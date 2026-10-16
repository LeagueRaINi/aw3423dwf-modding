# aw3423dwf-modding

`awtool` patches and flashes firmware for the **Alienware AW3423DWF** (FreeSync model, Realtek
RTD2717Q scaler, firmware **M3B107**). Its main use is raising the maximum refresh rate from 165 Hz
to 175 Hz.

This repository contains **no Dell firmware, binaries or keys**. You download Dell's official
firmware update yourself, and `awtool` builds the modified image from it on your machine.

> [!WARNING]
> **Flashing modified firmware can brick your monitor and voids your warranty.** The recovery path
> depends on reverse-engineered boot behaviour that has not been fully validated, and no complete
> factory-image backup procedure exists yet. You use this entirely at your own risk.
>
> This project is not affiliated with or endorsed by Dell or Alienware.
>
> It is **not** for the G-Sync AW3423DW, which uses different hardware.

## Profiles

| Profile | Status |
| --- | --- |
| [`profiles/175hz.toml`](profiles/175hz.toml) | Tested on hardware: 175 Hz, no artifacts or frame skipping |
| [`profiles/175hz-freesync.toml`](profiles/175hz-freesync.toml) | Same timing, with the AMD FreeSync range raised to 175 Hz; live VRR untested |
| [`profiles/177hz-freesync.toml`](profiles/177hz-freesync.toml) | Experimental: 177 Hz works on one unit; 178 Hz and above cause color cycling |

Rates above about 177 Hz failed in testing. The panel's output pixel clock tops out near the stock
1,021,760 kHz, so any further gain has to come from a shorter vertical blank, and 178 Hz and above
produced color cycling. Every profile shortens the output vertical blank compared with stock, and
the long-term effects of that on the panel are unknown.

All profiles also remap the monitor's product IDs (`DELA211…` → `DELA2E1…`), so Windows Update stops
installing Dell's monitor driver over the custom modes. They also rename the monitor to `AW3423DWF-CF`.

## Requirements

- Windows. Hardware access goes through Dell's 32-bit `RtHIDlib.dll`, so `awtool` builds for
  `i686-pc-windows-msvc` (this is set in `.cargo/config.toml`).
- To build from source: Rust 1.88 or newer, and `rustup target add i686-pc-windows-msvc`
- Dell's **AW3423DWF firmware update package for M3B107**, from Dell support. Extract it. You need
  - `M3B107\AW3423DWF_M3B107_20240130(ISP).UPG`, the stock firmware (input to `patch`)
  - `Firmware Updater.exe`, `RtHIDlib.dll` and the bundled VC++/MFC DLLs (used by `isp` and `flash`)
- A USB upstream cable from the monitor to the PC.

`awtool` stores only SHA-256 hashes of the updater's OEM handshake keys. At runtime it reads the
keys themselves from your copy of `Firmware Updater.exe`.

## Download or build

Prebuilt `awtool.exe` (32-bit, no runtime install needed) and the profiles are on the
[Releases](https://github.com/LeagueRaINi/aw3423dwf-modding/releases) page. To build it yourself:

```powershell
cargo build --release --locked
# binary: target\i686-pc-windows-msvc\release\awtool.exe
```

## Usage

### 1. Build a patched image

```powershell
$stock = 'C:\path\to\update\M3B107\AW3423DWF_M3B107_20240130(ISP).UPG'
awtool upg $stock                                   # structure + checksum check
awtool patch $stock AW3423DWF_175HZ.UPG --config profiles\175hz.toml
```

`patch` warns if the input is not the stock image
(`sha256 96b95afbefacacedb9d77f25fe142755088d3783e64c5a2deda40ec9c8ce7808`). It prints the
output's SHA-256, which should match the hash in the profile's header. Every firmware edit first
checks the exact stock bytes at its site, and if anything doesn't match, nothing is written.

### 2. Check the connection

Run these from the extracted updater folder, or pass `--updater-dir <folder>`:

```powershell
awtool isp info      # firmware version + panel id, no ISP entry
awtool isp crc       # per-64 KB hardware CRC of the whole flash
```

### 3. Flash

> [!IMPORTANT]
> **Before flashing, set Windows to 165 Hz or lower (60 Hz is safest).** After the flash, the
> scaler reboots straight into the new firmware while your GPU keeps sending the same mode. If that
> firmware doesn't accept the mode, you get a black screen or "input timing not supported" and
> have to change the resolution blind, or use another display, before you can do anything else.
> This applies in both directions: flashing a modded image, and flashing back to stock from 175 Hz.
> Only switch to the higher refresh rate after the monitor is back up on the new firmware.
>
> If you do end up with a black screen on DP, connect an HDMI cable as well (HDMI stays at 120 Hz
> in every profile), switch the monitor's input to HDMI, lower the DP display's refresh rate in
> Windows display settings, then switch back to DP. This can be finicky: Windows may need a moment,
> or a replug, before both connections show up.

```powershell
awtool flash AW3423DWF_175HZ.UPG          # dry run: lists the blocks that would change
awtool flash AW3423DWF_175HZ.UPG --arm    # write
```

What to expect:

- Entering ISP halts the scaler's MCU. The picture stays up, but the OSD, buttons and DDC freeze
  until the scaler reboots at the end. Do not power off or unplug during a flash.
- Before erasing anything, `--arm` reads the entire 64 KB "update complete" marker block byte by
  byte. At ISP speeds (about 11 B/s) this takes **roughly 100 minutes**. If the dry run reports
  the normal `AA55 present` state, `--trust-marker-crc` skips this read and trusts the CRC instead.
- Writes are limited to the user-image region and the marker block. The golden (factory) region is
  never erased. The order is: invalidate marker → self-test the write primitives on the marker
  block → erase, write and CRC-verify each changed 64 KB block → restore the marker → reboot.
- If the scaler stays frozen after an error, `awtool isp reboot` restarts it without using DDC.

Verification uses the scaler's hardware CRC-8, which is not a byte-for-byte proof.
`awtool isp diff <UPG> <SKIP>` compares actual bytes, but it is very slow.

To go back to stock, flash the stock UPG the same way.

### Backing up flash

`isp dump` reads any flash range, including the golden (factory/boot) image, and writes it to a
file. It checks every 4 KB chunk against the scaler's hardware CRC. If the dump is interrupted, run
the same command again: it re-verifies the chunks already saved and continues from where it
stopped. The ISP register guard rejects any erase or program command, so a dump cannot modify flash.

```powershell
awtool isp dump 0x000000 0x10000  golden_bank0.bin   # golden image, first 64 KB bank
awtool isp dump 0x000000 0x1C0000 golden_full.bin    # golden image, all 28 banks
awtool isp dump 0x400000 0x1C0000 user_image.bin     # currently running user image
```

ISP reads are very slow, about 11 B/s:

| Range | Size | Approx. time |
| --- | --- | --- |
| One 64 KB bank | 64 KB | ~1.5–2 hours |
| Golden or user image (`0x1C0000`) | 1.75 MB | ~2 days |
| Whole 8 MB chip | 8 MB | ~9 days |

The OSD and buttons are frozen while a dump runs. If you stop a dump with Ctrl+C, the scaler stays
halted; run `awtool isp reboot` to bring it back, then rerun the dump to continue. `isp read ADDR
LEN OUT` reads a small range in a single pass, without resume support.

Dumps contain Dell's firmware. Keep them for your own recovery and analysis, and don't publish them.

## Commands

| Command | Purpose |
| --- | --- |
| `upg FILE` | Container layout and checksum verification |
| `edid FILE` | List embedded EDIDs and their high-refresh timings |
| `crc FILE` | Per-64 KB CRC-8 of each image, comparable with `isp crc` |
| `flashview FILE PANEL OUT` | Write one panel's image as it sits in flash (signatures stripped, flash order) |
| `patch SRC DST --config PROFILE` | Apply a profile and fix every checksum |
| `flash UPG [--arm] [--trust-marker-crc]` | Flash over direct ISP; dry run unless `--arm` |
| `isp info\|crc\|read\|dump\|diff\|reboot` | Hardware reads and recovery |
| `ddc caps\|scan\|get` | Read-only DDC/CI through Windows' monitor API |
| `dis BANK [ENTRY…] [--case A] [--range S L]` | 8051 disassembly of one 64 KB bank |
| `xref FLASHVIEW xdata\|code ADDR` | Find XDATA accesses or code references across all banks |

Numbers accept decimal or `0x` hex. Bank-local code addresses are 16-bit. `flashview` offsets
are relative to the user image at flash `0x400000`. UPG file offsets include interleaved signed-bank
records, so don't use them as flash offsets.

## Writing a profile

Profiles are TOML, and every section is optional. Unknown keys, out-of-range values, unencodable
timings and missing patch targets are rejected before anything is written.

| Section | Effect |
| --- | --- |
| `[pid] from, to, count` | Remap a run of EDID product IDs |
| `[name] base, suffix` | Append a suffix to the EDID monitor name (12 chars max) |
| `[range] max_vhz [, max_h_khz, max_pclk_mhz]` | EDID range-limits descriptor |
| `[refresh] cap_hz` | Firmware refresh ceilings (input validators, DP output clamps, panel-output cap) |
| `[output] nominal_hz, clock_nominal_khz, clock_min_khz, clock_max_khz` | Panel-output refresh descriptor and clock window. Raising the clock above stock fails on hardware |
| `[panel] dv_total_min` | Output vertical-total floor (stock 1708, accepted 1456–1708) |
| `[input] max_h_khz` | Input horizontal-rate validator (stock 285 kHz) |
| `[freesync] dp_max_hz` | AMD FreeSync maximum in all six DP EDID templates (must equal `range.max_vhz`) |
| `[[mode]]` ×3 | DisplayID Type-I timings written into the three DID slots, in order |

The UPG's cryptographic signatures are not regenerated, and the modified banks keep their original
signature bytes. Direct ISP flashing bypasses the updater's file checks.

## Development

```powershell
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

The ignored tests need Dell's files and run with `--include-ignored`. They rebuild every profile
from stock and compare the result against the published hashes:

```powershell
$env:AWTOOL_STOCK_UPG   = 'C:\path\to\update\M3B107\AW3423DWF_M3B107_20240130(ISP).UPG'
$env:AWTOOL_UPDATER_DIR = 'C:\path\to\update'
cargo test --locked -- --include-ignored
```

| Module | Responsibility |
| --- | --- |
| `main.rs`, `cli.rs`, `commands/` | Entry point, arguments and command handlers |
| `config.rs` | Profile schema and validation |
| `upg.rs` | UPG container layout, checksums and flash-order conversion |
| `edid.rs` | EDID/DisplayID/CTA parsing, encoding and checksums |
| `firmware.rs`, `firmware/` | Exact M3B107 code-site anchors and atomic edits |
| `flash.rs` | Writable-region policy and marker-state validation |
| `isp.rs` | Authenticated I2C/ISP operations, register guards and session cleanup |
| `keys.rs` | Recovers the OEM handshake keys from Dell's updater |
| `hid.rs`, `rthid.rs` | Transport trait and the `RtHIDlib.dll` binding |
| `ddc.rs` | Read-only DDC/CI via `dxva2.dll` |
| `dis51.rs` | 8051 decoder and heuristic control-flow analysis |

## License

MIT; see [LICENSE](LICENSE). This covers this repository's code only. Dell's firmware, updater and
DLLs remain Dell's property and are not distributed here.
