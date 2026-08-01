# Fails the build when the release binary outgrows its stated bound.
#
# A tripwire, not a target. The limit sits above the measured size with room for a small
# feature, so that crossing it means something worth looking at rather than routine noise.
# The size is printed on every run either way, so growth is visible long before the limit.
#
# Shared by the CI and release workflows, so a tag can never publish something CI would
# have refused.

$ErrorActionPreference = 'Stop'

# 320 KiB: the measured size plus room for a small feature. This is the only place the
# ceiling is written down. README says that CI checks the size and leaves the number here,
# because a figure repeated in prose is a figure that drifts, and the line below prints the
# real one on every run for anyone who wants it.
$Limit = 327680
$Exe = 'target/release/inzone-h9-gen1-headset-status.exe'

if (-not (Test-Path $Exe)) {
    throw "no release binary at $Exe - run cargo build --release first"
}

$size = (Get-Item $Exe).Length
$kib = [math]::Round($size / 1KB, 1)
$limitKib = [math]::Round($Limit / 1KB, 1)
"release binary: $size bytes ($kib KiB), limit $Limit bytes ($limitKib KiB)"

if ($size -gt $Limit) {
    throw "the release binary is $($size - $Limit) bytes over the $limitKib KiB limit. " +
          "Either trim it, or raise `$Limit in this script, deliberately."
}

"$([math]::Round(($Limit - $size) / 1KB, 1)) KiB to spare"
