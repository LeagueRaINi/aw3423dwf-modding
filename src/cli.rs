//! Command-line arguments.

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "awtool",
    version,
    about = "firmware tooling for the Alienware AW3423DWF (Realtek RTD2717Q)"
)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Container layout + checksum verification.
    Upg { file: String },
    /// List embedded EDIDs and their high-refresh timings.
    Edid { file: String },
    /// Per-64K CRC-8 of each image (compare with `isp crc`).
    Crc { file: String },
    /// Write one panel's image as it sits in flash (signatures stripped, banks in flash order).
    Flashview {
        file: String,
        panel: String,
        out: String,
    },
    /// Apply the declarative patch spec to a UPG and fix every checksum.
    Patch {
        src: String,
        dst: String,
        /// TOML patch profile, e.g. profiles/175hz.toml.
        #[arg(long)]
        config: PathBuf,
    },
    /// Flash a patched UPG to the monitor over direct ISP (dry-run unless --arm).
    Flash {
        upg: String,
        /// Perform the destructive write (omit for a dry run).
        #[arg(long)]
        arm: bool,
        /// Skip the ~100-minute marker-block byte read when its CRC-8 shows the normal AA55-present
        /// state. Refused for any other state. CRC-8 cannot prove the rest of that block is erased.
        #[arg(long = "trust-marker-crc")]
        trust_marker_crc: bool,
        /// Dell updater folder holding RtHIDlib.dll and Firmware Updater.exe (default: search upward from the current directory).
        #[arg(long = "updater-dir", alias = "dll-dir")]
        updater_dir: Option<PathBuf>,
    },
    /// Read-only access to the monitor over its USB hub.
    Isp {
        #[command(subcommand)]
        sub: IspCmd,
        /// Dell updater folder holding RtHIDlib.dll and Firmware Updater.exe (default: search upward from the current directory).
        #[arg(long = "updater-dir", alias = "dll-dir", global = true)]
        updater_dir: Option<PathBuf>,
    },
    /// 8051 recursive-descent disassembly of one 64K bank (code address 0 = file offset 0).
    Dis {
        bank: String,
        /// Entry points (decimal or 0x-prefixed hex); defaults to 0 + linear-sweep targets.
        entries: Vec<String>,
        /// Treat ADDR (decimal or 0x-prefixed hex) as a jump-table helper; repeatable.
        #[arg(long = "case", value_name = "ADDR")]
        case: Vec<String>,
        /// Disassemble a raw range instead of tracing: START LEN (decimal or 0x-prefixed hex).
        #[arg(long, num_args = 2, value_names = ["START", "LEN"])]
        range: Option<Vec<String>>,
    },
    /// Cross-reference a whole flash-view image for XDATA accesses or code targets.
    Xref {
        file: String,
        kind: XrefKind,
        /// Address to search for (decimal or 0x-prefixed hex).
        addr: String,
    },
    /// Read-only DDC/CI (VCP) through dxva2.
    Ddc {
        #[command(subcommand)]
        sub: DdcCmd,
    },
}

#[derive(Subcommand)]
pub enum IspCmd {
    /// Firmware version + panel id (no ISP entry).
    Info,
    /// Per-64K CRC-8 of the whole flash.
    Crc,
    /// Read LEN bytes at ADDR to OUT, verified against the hardware CRC.
    Read {
        addr: String,
        len: String,
        out: String,
    },
    /// Resumable, per-4KB CRC-checked dump to OUT; re-run to continue.
    Dump {
        addr: String,
        len: String,
        out: String,
    },
    /// Find the first differing byte, reading flash in order (SKIP is a flash-view offset).
    Diff { upg: String, skip: String },
    /// Reboot the scaler without touching DDC (recovery).
    Reboot,
}

#[derive(Subcommand)]
pub enum DdcCmd {
    /// Read the VCP capabilities string.
    Caps,
    /// Probe every VCP code and list those that reply non-zero.
    Scan,
    /// Read one VCP code (decimal or 0x-prefixed hex).
    Get { code: String },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum XrefKind {
    /// MOV DPTR,#ADDR sites, classified read/write.
    Xdata,
    /// LCALL/LJMP/ACALL/AJMP targeting ADDR.
    Code,
}
