//! The Sony HCI frame format the dongle speaks, and the vocabulary of its payloads.
//!
//! Command (PC -> dongle), 4-byte HCI header then data:
//!   01 | 00 FC | param_len | 96 C3 | (dst<<4)|src | event_id | event_type | tid_lo tid_hi | checksum
//! Event (dongle -> PC), 3-byte HCI header then data:
//!   04 | FF | param_len | 00 | 96 C3 | (dst<<4)|src | event_id | event_type | tid_lo tid_hi | params.. | checksum
//! checksum = sum of every data byte (i.e. after the HCI header), excluding the checksum slot.

const SRC_PC: u8 = 0x01;
pub const DST_TX: u8 = 0x02; // the dongle
pub const DST_RX: u8 = 0x04; // the headset itself

/// A fixed two bytes that open every frame's body in both directions. Named once so the
/// encoder and the decoder cannot come to disagree.
const SONY_KEY: [u8; 2] = [0x96, 0xC3];

pub const EV_2GHZ_CONNECT_STATUS: u8 = 0x01;
pub const EV_BATTERY_INFO: u8 = 0x04;
pub const EV_BT_STATUS: u8 = 0x61;
// Event `0x02` addressed to the headset is one another implementation sends, ahead of its
// battery query, and this one has never needed. Its meaning is unknown; a session or a
// wake-up would fit. Worth trying if a first poll after a resume is ever answered with
// nothing, and worth confirming on hardware before it earns a name here.
const TYPE_GET: u8 = 0x01;
const TYPE_RET: u8 = 0x10;

/// An event body is dummy + key + address + event id + event type + transaction id +
/// checksum, so nothing shorter than this is a frame.
const HEADSET_DATA_LENGTH_MIN: usize = 9;

/// The dongle's link to the headset.
pub const CONN_UNCONNECTED: u8 = 0x00;
pub const CONN_CONNECTED: u8 = 0x01;
pub const CONN_PAIRING: u8 = 0x02;

/// The charge state byte. `ERROR = 0x02` is deliberately absent: it is handled by not
/// matching, which drops the level to unknown.
pub const BATT_DISCHARGING: u8 = 0x00;
pub const BATT_CHARGING: u8 = 0x01;

/// The first byte of an EV_BT_STATUS reply: whether Bluetooth itself is on.
pub const BT_OFF: u8 = 0x00;
pub const BT_ON: u8 = 0x01;
/// The second byte: whether anything is connected over it.
pub const BT_CONN_UNCONNECTED: u8 = 0x00;
pub const BT_CONN_CONNECTED: u8 = 0x01;
pub const BT_CONN_PAIRING: u8 = 0x02;

pub fn encode_get(dst: u8, event_id: u8, tid: u16) -> Vec<u8> {
    let data = [
        SONY_KEY[0],
        SONY_KEY[1],
        (dst << 4) | SRC_PC,
        event_id,
        TYPE_GET,
        tid as u8,
        (tid >> 8) as u8,
    ];
    let mut frame = vec![0x01, 0x00, 0xFC, data.len() as u8 + 1];
    frame.extend_from_slice(&data);
    frame.push(checksum(&data));
    frame
}

fn checksum(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |a, &b| a.wrapping_add(b))
}

/// Returns the parameter bytes of a matching RET event, or an error describing why not.
fn decode_ret(buf: &[u8], event_id: u8, tid: u16) -> Result<&[u8], String> {
    if buf.len() < 3 {
        return Err(format!("short reply ({} bytes)", buf.len()));
    }
    if buf[0] != 0x04 || buf[1] != 0xFF {
        return Err(format!("not a vendor HCI event: {:02X} {:02X}", buf[0], buf[1]));
    }
    // The length byte comes off the wire, so it decides how much of the rest of this
    // function is even meaningful. Anything below the protocol's own minimum cannot carry
    // a body, and rejecting it here is what keeps the split_last below total: at 0 it used
    // to panic, which in a release build aborts the process and leaves a ghost tray icon.
    let param_len = buf[2] as usize;
    if param_len < HEADSET_DATA_LENGTH_MIN {
        return Err(format!(
            "param_len {param_len} below the {HEADSET_DATA_LENGTH_MIN} byte minimum"
        ));
    }
    let end = 3 + param_len;
    if buf.len() < end {
        return Err(format!("truncated: want {end} bytes, have {}", buf.len()));
    }
    let data = &buf[3..end];
    let (&cs, body) = data.split_last().expect("param_len >= 9 guarantees a body");
    if checksum(body) != cs {
        return Err("checksum mismatch".into());
    }
    // body: 00 | 96 C3 | addr | event_id | event_type | tid_lo tid_hi | params..
    // param_len >= 9 makes body at least 8 bytes, so every index below is in range.
    //
    // The key is checked and the address is not, deliberately. The key is two fixed bytes
    // confirmed in both directions: `encode_get` writes them, and every captured reply
    // opens with them. The address encodes which end sent the frame, and the only frames
    // ever captured came from one headset on one path, with H7, both pairing states and
    // charging still unconfirmed. Asserting on it would risk rejecting a valid reply from
    // a path nobody has seen yet, which is a worse failure than the one it would prevent.
    if body[1..3] != SONY_KEY {
        return Err(format!("not a Sony frame: {:02X} {:02X}", body[1], body[2]));
    }
    if body[4] != event_id {
        return Err(format!("event id {:02X} != {event_id:02X}", body[4]));
    }
    if body[5] != TYPE_RET {
        return Err(format!("event type {:02X} is not RET", body[5]));
    }
    let got_tid = u16::from_le_bytes([body[6], body[7]]);
    if got_tid != tid {
        return Err(format!("transaction id {got_tid} != {tid}"));
    }
    Ok(&body[8..])
}

/// Whether an incomplete frame at the front of the buffer is still worth waiting for.
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum Scan {
    MoreMayCome,
    StreamEnded,
}

/// Pulls complete frames off the front of `buf` until one answers the request, consuming
/// everything it rejects. Returns `None` when more bytes are needed. Frames that fail to
/// decode leave their reason in `last_err` rather than ending the exchange, because the
/// protocol has notification event types and something we did not ask for can land ahead of
/// the answer we did.
pub fn take_answer(
    buf: &mut Vec<u8>,
    event_id: u8,
    tid: u16,
    last_err: &mut Option<String>,
    mode: Scan,
) -> Option<Vec<u8>> {
    while buf.len() >= 3 {
        // The length byte only means anything once the two header bytes agree, so a stream
        // that starts mid-frame is resynchronized a byte at a time. Trusting buf[2] while
        // misaligned would let a single stray byte swallow the reply sitting behind it.
        if buf[0] != 0x04 || buf[1] != 0xFF {
            buf.drain(..1);
            continue;
        }
        let end = 3 + buf[2] as usize;
        if buf.len() < end {
            // Waiting is only reasonable while bytes may still arrive. Two stray bytes can
            // pose as a header and claim a 258 byte frame that will never come; once the
            // port has gone quiet, that claim is disproved and the byte was never a length.
            if mode == Scan::MoreMayCome {
                return None;
            }
            buf.drain(..1);
            continue;
        }
        match decode_ret(&buf[..end], event_id, tid) {
            Ok(params) => {
                let params = params.to_vec();
                buf.drain(..end);
                return Some(params);
            }
            Err(e) => {
                // Keep the first reason. Byte-stepping walks through the interior of a
                // frame it rejected, and those interior misreads are far less informative
                // than the decode that actually failed on a real frame boundary.
                last_err.get_or_insert(e);
                // Only a frame that decoded proves where its boundary was. On failure the
                // length byte may not have been a length at all (two stray bytes can look
                // like a header), so step a single byte and look again rather than trusting
                // it to skip exactly the right amount and eating the reply behind it.
                buf.drain(..1);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference frames, and a real reply captured from the dongle. If the encoder or
    /// the field order ever drifts, these fail.
    #[test]
    fn encodes_the_reference_frames() {
        assert_eq!(
            encode_get(DST_TX, EV_2GHZ_CONNECT_STATUS, 1),
            vec![0x01, 0x00, 0xFC, 0x08, 0x96, 0xC3, 0x21, 0x01, 0x01, 0x01, 0x00, 0x7D]
        );
        assert_eq!(
            encode_get(DST_RX, EV_BATTERY_INFO, 3),
            vec![0x01, 0x00, 0xFC, 0x08, 0x96, 0xC3, 0x41, 0x04, 0x01, 0x03, 0x00, 0xA2]
        );
        // A transaction id past 255, which the poller reaches after about sixteen minutes:
        // it steps by four every fifteen seconds and wraps at sixteen bits. The high byte is
        // the one field the two frames above cannot exercise, and losing it is not a quiet
        // failure. `decode_ret` checks the id the dongle echoes against the whole `u16` it
        // was handed, so a high byte written as zero makes every reply from that point on
        // look like somebody else's and the tray stops moving.
        assert_eq!(
            encode_get(DST_TX, EV_2GHZ_CONNECT_STATUS, 0x1234),
            vec![0x01, 0x00, 0xFC, 0x08, 0x96, 0xC3, 0x21, 0x01, 0x01, 0x34, 0x12, 0xC2]
        );
    }

    #[test]
    fn decodes_a_real_reply() {
        let connected = [
            0x04, 0xFF, 0x0A, 0x00, 0x96, 0xC3, 0x12, 0x01, 0x10, 0x01, 0x00, 0x01, 0x7E,
        ];
        assert_eq!(
            decode_ret(&connected, EV_2GHZ_CONNECT_STATUS, 1),
            Ok(&[CONN_CONNECTED][..])
        );

        // Captured with the headset powered off: same frame, last param 0x00.
        let off = [
            0x04, 0xFF, 0x0A, 0x00, 0x96, 0xC3, 0x12, 0x01, 0x10, 0x01, 0x00, 0x00, 0x7D,
        ];
        assert_eq!(decode_ret(&off, EV_2GHZ_CONNECT_STATUS, 1), Ok(&[CONN_UNCONNECTED][..]));

        let battery = [
            0x04, 0xFF, 0x0B, 0x00, 0x96, 0xC3, 0x14, 0x04, 0x10, 0x03, 0x00, 0x00, 0x64, 0xE8,
        ];
        assert_eq!(decode_ret(&battery, EV_BATTERY_INFO, 3), Ok(&[0x00, 100][..]));

        // Bluetooth on and connected. Two bytes: BT_STATUS then BT_CONNECTION_STATUS.
        let bt = [
            0x04, 0xFF, 0x0B, 0x00, 0x96, 0xC3, 0x14, 0x61, 0x10, 0x02, 0x00, 0x01, 0x01, 0xE2,
        ];
        assert_eq!(decode_ret(&bt, EV_BT_STATUS, 2), Ok(&[BT_ON, BT_CONN_CONNECTED][..]));
    }

    /// `decode_ret` runs on bytes straight off the wire, so it has to be total. A
    /// `param_len` of 0 used to reach `split_last().unwrap()` and panic, which under
    /// `panic = "abort"` kills the process and leaves a ghost icon in the tray.
    #[test]
    fn survives_any_length_byte() {
        for param_len in 0u8..=255 {
            for extra in [0usize, 1, 12, 40] {
                let mut frame = vec![0x04u8, 0xFF, param_len];
                frame.resize(3 + extra, 0xAA);
                // Filler can never be a legal reply: even where the sum happens to match,
                // body[4] is 0xAA and never the event id. So nothing here may decode.
                assert!(
                    decode_ret(&frame, EV_2GHZ_CONNECT_STATUS, 1).is_err(),
                    "param_len {param_len} extra {extra} decoded from filler"
                );
            }
        }
        // Below the protocol minimum it must refuse rather than reach for a body.
        for param_len in 0u8..HEADSET_DATA_LENGTH_MIN as u8 {
            let mut frame = vec![0x04u8, 0xFF, param_len];
            frame.resize(32, 0);
            assert!(
                decode_ret(&frame, EV_2GHZ_CONNECT_STATUS, 1).is_err(),
                "param_len {param_len}"
            );
        }
    }

    /// Every truncation and single-byte mutation of a known-good frame must come back as an
    /// error, never a panic and never a wrong success.
    #[test]
    fn survives_truncation_and_mutation() {
        let good = [
            0x04u8, 0xFF, 0x0A, 0x00, 0x96, 0xC3, 0x12, 0x01, 0x10, 0x01, 0x00, 0x01, 0x7E,
        ];
        assert!(decode_ret(&good, EV_2GHZ_CONNECT_STATUS, 1).is_ok());

        for cut in 0..good.len() {
            let _ = decode_ret(&good[..cut], EV_2GHZ_CONNECT_STATUS, 1);
        }
        // The checksum covers the whole body, payload included, so today every single-bit
        // flip is rejected. This is a tripwire rather than a demonstration: if the checksum
        // ever stops covering part of the frame, some flip will start decoding and this
        // fires with the byte that slipped through.
        for i in 0..good.len() {
            for bit in 0..8 {
                let mut bad = good;
                bad[i] ^= 1 << bit;
                assert!(
                    decode_ret(&bad, EV_2GHZ_CONNECT_STATUS, 1).is_err(),
                    "byte {i} bit {bit} still decoded"
                );
            }
        }
    }

    /// A frame we did not ask for must not sink the whole exchange: the scan drops it and
    /// keeps looking for the answer behind it.
    #[test]
    fn skips_frames_that_are_not_the_answer() {
        // A well-formed BT_STATUS notification: right shape, right checksum, wrong event.
        // This is the frame the resync exists for, so its checksum has to be correct or the
        // test proves nothing beyond "bad checksums are dropped".
        let ntfy = [
            0x04u8, 0xFF, 0x0A, 0x00, 0x96, 0xC3, 0x12, 0x61, 0x20, 0x09, 0x00, 0x01, 0xF6,
        ];
        // It must be rejected for being a notification, not for being malformed. If the
        // checksum were wrong the test would only prove that bad frames get dropped.
        let why = decode_ret(&ntfy, EV_BT_STATUS, 9).unwrap_err();
        assert!(why.contains("RET"), "the noise frame must be well formed, but: {why}");

        let wanted = [
            0x04u8, 0xFF, 0x0A, 0x00, 0x96, 0xC3, 0x12, 0x01, 0x10, 0x01, 0x00, 0x01, 0x7E,
        ];
        let mut buf = Vec::new();
        buf.push(0x5A); // a stray byte, so the scan has to resynchronize before anything else
        buf.extend_from_slice(&[0x04, 0xFF, 0x00]); // the length byte that used to panic
        buf.extend_from_slice(&ntfy);
        buf.extend_from_slice(&wanted);

        let mut last_err = None;
        let found = take_answer(&mut buf, EV_2GHZ_CONNECT_STATUS, 1, &mut last_err, Scan::MoreMayCome);
        assert_eq!(
            found,
            Some(vec![CONN_CONNECTED]),
            "the real reply must survive the noise"
        );
        assert!(buf.is_empty(), "the answer and everything before it must be consumed");
        assert!(last_err.is_some(), "the skipped frames must leave a reason behind");
    }

    /// A misaligned stream must not let a garbage length byte eat the reply behind it.
    #[test]
    fn resynchronizes_a_byte_at_a_time() {
        let wanted = [
            0x04u8, 0xFF, 0x0A, 0x00, 0x96, 0xC3, 0x12, 0x01, 0x10, 0x01, 0x00, 0x01, 0x7E,
        ];
        // A well-formed frame for a different event, whose payload happens to contain a
        // header-shaped pair followed by a large byte. Byte-stepping walks through frames
        // it rejects, so a payload is a scan position too.
        let foreign = [
            0x04u8, 0xFF, 0x0C, 0x00, 0x96, 0xC3, 0x12, 0x61, 0x10, 0x09, 0x00, 0x04, 0xFF, 0xF0, 0xD8,
        ];
        assert!(
            decode_ret(&foreign, EV_BT_STATUS, 9).is_ok(),
            "the decoy must itself be valid"
        );

        let prefixes: [Vec<u8>; 6] = [
            vec![0xFFu8],
            vec![0x04, 0xFF],
            vec![0x00; 5],
            vec![0x04, 0x04, 0xFF],
            // The genuinely nasty one: a false header whose length slot claims 240 bytes
            // that will never arrive. Waiting for it strands the answer sitting behind it.
            vec![0x04, 0xFF, 0xF0],
            foreign.to_vec(),
        ];
        for prefix in prefixes {
            let mut buf = prefix.clone();
            buf.extend_from_slice(&wanted);
            let mut last_err = None;
            assert_eq!(
                take_answer(&mut buf, EV_2GHZ_CONNECT_STATUS, 1, &mut last_err, Scan::StreamEnded),
                Some(vec![CONN_CONNECTED]),
                "prefix {prefix:02X?} swallowed the reply"
            );
        }

        // *Both* header bytes have to agree before the length byte means anything, and the
        // cases above do not prove it: once the stream is over, a length nothing satisfies
        // gets stepped past anyway and the answer is found regardless. It is while bytes may
        // still arrive that a half-matching header costs the reply. `04 AA` is not a header,
        // so the scan must walk past it rather than wait out the 240 bytes the next byte
        // claims, which nothing is going to send.
        let mut buf = vec![0x04u8, 0xAA, 0xF0];
        buf.extend_from_slice(&wanted);
        let mut last_err = None;
        assert_eq!(
            take_answer(&mut buf, EV_2GHZ_CONNECT_STATUS, 1, &mut last_err, Scan::MoreMayCome),
            Some(vec![CONN_CONNECTED]),
            "a header that matches on one byte must not be trusted for its length"
        );
    }

    /// The two scan modes have to disagree exactly where it matters: while bytes may still
    /// arrive an incomplete frame is kept, and once the port is quiet it is disproved.
    #[test]
    fn waits_for_an_incomplete_frame_instead_of_dropping_it() {
        let wanted = [
            0x04u8, 0xFF, 0x0A, 0x00, 0x96, 0xC3, 0x12, 0x01, 0x10, 0x01, 0x00, 0x01, 0x7E,
        ];
        for cut in 3..wanted.len() {
            let mut buf = wanted[..cut].to_vec();
            let mut last_err = None;
            let waiting = take_answer(&mut buf, EV_2GHZ_CONNECT_STATUS, 1, &mut last_err, Scan::MoreMayCome);
            assert_eq!(waiting, None);
            assert_eq!(buf.len(), cut, "a partial frame must be kept, not consumed");
            buf.extend_from_slice(&wanted[cut..]);
            assert_eq!(
                take_answer(&mut buf, EV_2GHZ_CONNECT_STATUS, 1, &mut last_err, Scan::MoreMayCome),
                Some(vec![CONN_CONNECTED])
            );
        }

        // A phantom length is only refuted once the stream stops.
        let mut buf = vec![0x04u8, 0xFF, 0xF0];
        buf.extend_from_slice(&wanted);
        let mut last_err = None;
        assert_eq!(
            take_answer(&mut buf, EV_2GHZ_CONNECT_STATUS, 1, &mut last_err, Scan::MoreMayCome),
            None,
            "while bytes may still come, the claimed length gets the benefit of the doubt"
        );
        assert_eq!(
            take_answer(&mut buf, EV_2GHZ_CONNECT_STATUS, 1, &mut last_err, Scan::StreamEnded),
            Some(vec![CONN_CONNECTED]),
            "once quiet, the reply behind the phantom must be salvaged"
        );
    }

    #[test]
    fn rejects_corrupt_and_mismatched_replies() {
        let mut bad = [
            0x04, 0xFF, 0x0A, 0x00, 0x96, 0xC3, 0x12, 0x01, 0x10, 0x01, 0x00, 0x01, 0x7E,
        ];
        bad[11] = 0x00; // flip a payload byte, leave the checksum stale
        assert!(decode_ret(&bad, EV_2GHZ_CONNECT_STATUS, 1).is_err());

        let good = [
            0x04, 0xFF, 0x0A, 0x00, 0x96, 0xC3, 0x12, 0x01, 0x10, 0x01, 0x00, 0x01, 0x7E,
        ];
        assert!(
            decode_ret(&good, EV_BATTERY_INFO, 1).is_err(),
            "wrong event id must not pass"
        );
        assert!(
            decode_ret(&good, EV_2GHZ_CONNECT_STATUS, 9).is_err(),
            "stale transaction must not pass"
        );

        // The key is what says a frame is Sony's at all, so the checksum is recomputed
        // here: the frame has to be turned away for the key rather than for arithmetic.
        let mut foreign = good;
        foreign[4] = 0x00;
        foreign[12] = checksum(&foreign[3..12]);
        assert!(
            decode_ret(&foreign, EV_2GHZ_CONNECT_STATUS, 1).is_err(),
            "a frame carrying someone else's key must not pass on its own checksum"
        );
    }
}
