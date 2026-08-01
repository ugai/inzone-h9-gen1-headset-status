//! Finding the dongle's virtual COM port and running one request/reply exchange over it.
//!
//! The port is opened and closed on every poll, because holding it would lock INZONE Hub out.

use std::time::{Duration, Instant};

use crate::protocol::{
    self, CONN_CONNECTED, CONN_PAIRING, CONN_UNCONNECTED, DST_RX, DST_TX, EV_2GHZ_CONNECT_STATUS, EV_BATTERY_INFO,
    EV_BT_STATUS, Scan, encode_get,
};
use crate::state::{Cause, State, interpret_battery, interpret_bt};

const VID: u16 = 0x054C;
const PID: u16 = 0x0E53;

/// Upper bound on one request and reply, including any frames skipped along the way. The
/// check precedes each read, so a read dispatched just under the wire can add its own
/// timeout on top: three queries put the worst case around 8 s of port occupancy.
const QUERY_DEADLINE: Duration = Duration::from_secs(2);

/// `Ok(None)` is a listing we read to the end, every entry of which we could identify, and
/// the dongle was not among them.
///
/// The two ways of not getting that far are errors rather than absences. An enumeration that
/// failed is one. The other is subtler: `serialport` resolves a composite device's VID and
/// PID through its *parent's* hardware id, and this dongle's port always sits under an
/// interface, so that lookup always runs. When it comes back empty the port is still listed,
/// but as `Unknown`, and a match on VID and PID alone walks straight past the dongle. Ports
/// found only in the registry are listed as `Unknown` too.
///
/// Reporting "no receiver plugged in" off the back of either would be the same class of lie
/// as reading a Bluetooth badge we never asked for as "Bluetooth is off", so the count of
/// entries we could not identify travels back with the answer and ends up in the tooltip.
fn find_port() -> Result<(Option<String>, usize), Cause> {
    let ports = serialport::available_ports().map_err(|e| Cause::Enumerate(e.description.clone()))?;
    Ok(choose_port(ports))
}

/// The dongle's port if it is on the list, and how many entries could not be identified at
/// all. Kept apart from the enumeration so it can be tested against a list built by hand;
/// the I/O around it is exercised only on real hardware, as the rest of this module is.
fn choose_port(ports: Vec<serialport::SerialPortInfo>) -> (Option<String>, usize) {
    let mut unidentified = 0usize;
    for p in ports {
        match p.port_type {
            serialport::SerialPortType::UsbPort(usb) if usb.vid == VID && usb.pid == PID => {
                return (Some(p.port_name), unidentified);
            }
            serialport::SerialPortType::Unknown => unidentified += 1,
            _ => {}
        }
    }
    (None, unidentified)
}

fn query(port: &mut dyn serialport::SerialPort, dst: u8, event_id: u8, tid: u16) -> Result<Vec<u8>, Cause> {
    port.clear(serialport::ClearBuffer::Input).ok();
    // No flush. `serialport` writes with a bare `WriteFile`, so there is no buffer of its
    // own between us and the driver and nothing for a flush to push out. What its `flush`
    // does call is `FlushFileBuffers`, and on a communications device that one waits for the
    // transmit queue to drain under flow control, outside the write timeout entirely. It is
    // the only call in the exchange `QUERY_DEADLINE` could not bound, and it was buying
    // nothing.
    port.write_all(&encode_get(dst, event_id, tid))
        .map_err(|e| Cause::Io(e.to_string()))?;

    // Replies arrive split across USB packet boundaries, so accumulate until the length
    // byte's promise is met. A frame that does not decode is dropped and the scan carries
    // on rather than failing the whole poll: the protocol has notification event types, so
    // something we did not ask for can land ahead of the answer we did.
    let mut buf: Vec<u8> = Vec::with_capacity(32);
    let mut chunk = [0u8; 64];
    let mut last_err: Option<String> = None;
    // Counted separately from `buf`, which the scan drains as it rejects: by the time the
    // exchange gives up, `buf` holds the two bytes too short to be a frame and nothing else.
    // A dongle that said nothing and one that streamed a kilobyte of rubbish would otherwise
    // report the same thing, and telling those apart is the whole point of the number.
    let mut received = 0usize;
    // Every successful read refreshes the port's own timeout, so a device that streams
    // rubbish without pause would keep this loop alive forever and hang the poller with it.
    // The deadline bounds the whole exchange rather than the gap between two reads.
    let deadline = Instant::now() + QUERY_DEADLINE;

    loop {
        if let Some(params) = protocol::take_answer(&mut buf, event_id, tid, &mut last_err, Scan::MoreMayCome) {
            return Ok(params);
        }
        let quiet = Instant::now() >= deadline;
        if !quiet {
            match port.read(&mut chunk) {
                Ok(0) => return Err(Cause::PortClosed),
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    received += n;
                    continue;
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) => return Err(Cause::Io(e.to_string())),
            }
        }
        // Nothing more is coming. Rescan without the benefit of the doubt: an answer may
        // be sitting behind a phantom length that will never be satisfied.
        if let Some(params) = protocol::take_answer(&mut buf, event_id, tid, &mut last_err, Scan::StreamEnded) {
            return Ok(params);
        }
        // Whatever we saw, said as a cause rather than a sentence: the tray decides the
        // wording, and this layer must not claim more than "nothing usable arrived".
        return Err(Cause::NoReply {
            detail: last_err,
            unusable: received,
        });
    }
}

/// serialport folds `ERROR_ACCESS_DENIED`, `ERROR_FILE_NOT_FOUND` and
/// `ERROR_PATH_NOT_FOUND` into a single `NoDevice` kind, so the kind alone cannot tell
/// "another process holds it" from "it is gone". `find_port()` listed this port a moment
/// earlier, so a refusal here means we could not acquire it, and the only thing that takes
/// this port exclusively is another process. If the receiver really was unplugged inside
/// that window, the next poll reports `NoDongle` and corrects the display.
fn classify_open_error(e: &serialport::Error) -> State {
    match e.kind() {
        serialport::ErrorKind::NoDevice => State::HubBusy,
        _ => State::Error(Cause::Open(e.description.clone())),
    }
}

pub fn poll(tid: u16) -> State {
    let name = match find_port() {
        Ok((Some(name), _)) => name,
        Ok((None, unidentified)) => return State::NoDongle { unidentified },
        Err(cause) => return State::Error(cause),
    };
    let mut port = match serialport::new(&name, 115_200)
        .timeout(Duration::from_millis(600))
        .open()
    {
        Ok(p) => p,
        Err(e) => return classify_open_error(&e),
    };
    // The dongle is a USB CDC device: it will not service the OUT endpoint until DTR is
    // asserted. Writing before that wedges the dongle's firmware until it is replugged,
    // so this is the one place in the program that must not continue on error. Give up the
    // poll instead: a missed reading costs 15 seconds, a wedged dongle costs a replug.
    if let Err(e) = port.write_data_terminal_ready(true) {
        return State::Error(Cause::Dtr(e.to_string()));
    }
    // RTS is not gating the endpoint, so a failure here is not worth abandoning the poll.
    port.write_request_to_send(true).ok();
    // 50 ms has been enough on every exchange this has been run through. Another
    // implementation of the same job settles for 500 ms, so if a first poll after a replug
    // or a resume ever comes back empty, this is the number to try before anything else.
    std::thread::sleep(Duration::from_millis(50));

    let conn = match query(&mut *port, DST_TX, EV_2GHZ_CONNECT_STATUS, tid) {
        Ok(p) if !p.is_empty() => p[0],
        Ok(_) => return State::Error(Cause::EmptyReply),
        Err(e) => return State::Error(e),
    };
    match conn {
        CONN_UNCONNECTED => State::Off,
        CONN_PAIRING => State::Pairing,
        CONN_CONNECTED => {
            // Both of these are addressed to the headset rather than the dongle, so a
            // failure means "it did not tell us", not "the answer is no".
            let batt = query(&mut *port, DST_RX, EV_BATTERY_INFO, tid.wrapping_add(1)).ok();
            let (battery, charging) = interpret_battery(batt.as_deref().unwrap_or(&[]));
            let bt = query(&mut *port, DST_RX, EV_BT_STATUS, tid.wrapping_add(2))
                .ok()
                .and_then(|p| interpret_bt(&p));
            State::On { battery, charging, bt }
        }
        other => State::Error(Cause::UnknownLink(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A port we could not identify must not be counted as the dongle's absence.
    ///
    /// `serialport` resolves a composite device's VID and PID through its parent, and this
    /// dongle's port always sits under an interface, so that lookup always runs. When it
    /// fails the port is still listed, as `Unknown`. Matching on VID and PID alone then
    /// walks past the dongle and `NoDongle` says "receiver not found" with a receiver
    /// plugged in, which is the confident wrong answer the rest of this program is built to
    /// avoid. Ports discovered only through the registry are `Unknown` as well.
    #[test]
    fn a_port_we_could_not_identify_is_not_an_absent_dongle() {
        use serialport::{SerialPortInfo, SerialPortType, UsbPortInfo};

        let usb = |vid, pid| {
            SerialPortType::UsbPort(UsbPortInfo {
                vid,
                pid,
                serial_number: None,
                manufacturer: None,
                product: None,
            })
        };
        let port = |name: &str, port_type| SerialPortInfo {
            port_name: name.into(),
            port_type,
        };

        assert_eq!(
            choose_port(vec![port("COM10", usb(VID, PID))]),
            (Some("COM10".into()), 0)
        );
        // Every entry identified, none of them ours: the dongle really is not there.
        assert_eq!(choose_port(vec![port("COM3", usb(0x0403, 0x6001))]), (None, 0));
        // Half the pair is not the pair. A vendor id on its own would take any other serial
        // device Sony makes and write to it, and a product id on its own is not unique at
        // all. Both halves are checked here because either one alone still passes the case
        // above, where neither matches.
        assert_eq!(choose_port(vec![port("COM3", usb(VID, 0x0001))]), (None, 0));
        assert_eq!(choose_port(vec![port("COM3", usb(0x0001, PID))]), (None, 0));
        assert_eq!(choose_port(vec![]), (None, 0));
        // The one this test exists for.
        assert_eq!(choose_port(vec![port("COM10", SerialPortType::Unknown)]), (None, 1));
        assert_eq!(
            choose_port(vec![
                port("COM3", usb(0x0403, 0x6001)),
                port("COM10", SerialPortType::Unknown),
                port("COM11", SerialPortType::Unknown),
            ]),
            (None, 2)
        );
        // A port we did identify as the dongle settles it, whatever else is on the list.
        assert_eq!(
            choose_port(vec![
                port("COM10", SerialPortType::Unknown),
                port("COM11", usb(VID, PID)),
            ]),
            (Some("COM11".into()), 1)
        );
    }

    /// A refused open must read as "the Hub has it", not as an error. serialport reports
    /// ACCESS_DENIED with the same `NoDevice` kind it uses for a missing port, so matching
    /// on `Io(PermissionDenied)` never fires and the icon went dim gray instead of violet.
    #[test]
    fn a_refused_port_reads_as_hub_busy() {
        use serialport::{Error, ErrorKind};
        assert_eq!(
            classify_open_error(&Error::new(ErrorKind::NoDevice, "アクセスが拒否されました。")),
            State::HubBusy
        );
        // Anything else is a genuine fault and must keep its message.
        let other = Error::new(ErrorKind::InvalidInput, "bad baud");
        assert_eq!(
            classify_open_error(&other),
            State::Error(Cause::Open("bad baud".into()))
        );
        assert!(matches!(
            classify_open_error(&Error::new(ErrorKind::Unknown, "?")),
            State::Error(_)
        ));
    }

    /// Dumps the raw reply bytes so a newly observed state can be pasted into
    /// `decodes_a_real_reply` verbatim.
    fn raw_exchange(dst: u8, event_id: u8, tid: u16) -> String {
        use std::io::{Read, Write};
        let name = match find_port() {
            Ok((Some(name), _)) => name,
            Ok((None, n)) => return format!("no port ({n} unidentified)"),
            Err(cause) => return format!("{cause:?}"),
        };
        let Ok(mut port) = serialport::new(&name, 115_200)
            .timeout(Duration::from_millis(600))
            .open()
        else {
            return "port busy".into();
        };
        // Same rule as poll(): writing without DTR wedges the dongle until it is
        // physically replugged, so a failure here has to stop before any byte goes out.
        // This helper runs against the real device, which is exactly where it matters.
        if let Err(e) = port.write_data_terminal_ready(true) {
            return format!("DTR failed, refusing to write: {e}");
        }
        port.write_request_to_send(true).ok();
        std::thread::sleep(Duration::from_millis(50));
        if port.write_all(&encode_get(dst, event_id, tid)).is_err() {
            return "write failed".into();
        }
        // No flush, for the reason query() gives: there is nothing buffered to push, and the
        // call would be the one part of the exchange with no bound on how long it can take.
        // Accumulate until the length byte's promise is met. Unlike query() this keeps the
        // first complete frame whatever it decodes to, because the job here is to show the
        // bytes rather than to find a particular reply.
        let mut got = Vec::new();
        let mut chunk = [0u8; 64];
        loop {
            match port.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    got.extend_from_slice(&chunk[..n]);
                    if got.len() >= 3 && got.len() >= 3 + got[2] as usize {
                        break;
                    }
                }
                Err(e) => {
                    if got.is_empty() {
                        return format!("read failed: {e}");
                    }
                    break;
                }
            }
        }
        got.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
    }

    /// Talks to the real dongle. Needs INZONE Hub closed, so it is opt-in:
    ///   cargo test -- --ignored --nocapture
    #[test]
    #[ignore]
    fn talks_to_the_real_dongle() {
        println!("port: {:?}", find_port());
        println!(
            "raw 2GHZ_CONNECT_STATUS: {}",
            raw_exchange(DST_TX, EV_2GHZ_CONNECT_STATUS, 1)
        );
        println!("raw BATTERY_INFO       : {}", raw_exchange(DST_RX, EV_BATTERY_INFO, 3));
        let state = poll(11);
        println!("state: {state:?}  tooltip: {}", state.tooltip(crate::text::Lang::Ja));
        assert!(
            !matches!(state, State::Error(_) | State::NoDongle { .. }),
            "expected a real answer, got {state:?}"
        );
    }
}
