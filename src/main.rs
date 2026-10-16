//! Firmware tooling for the Alienware AW3423DWF (Realtek RTD2717Q).

mod cli;
mod commands;
mod config;
mod crc8;
mod ddc;
mod dis51;
mod edid;
mod firmware;
mod flash;
mod hid;
mod isp;
mod keys;
mod rthid;
mod upg;

#[cfg(test)]
mod fixture_tests;

use clap::Parser;
use cli::{Cli, Cmd};
use commands::*;
use std::process::ExitCode;

fn main() -> ExitCode {
    let r = match Cli::parse().cmd {
        Cmd::Upg { file } => cmd_upg(&file),
        Cmd::Edid { file } => cmd_edid(&file),
        Cmd::Crc { file } => cmd_crc(&file),
        Cmd::Flashview { file, panel, out } => cmd_flashview(&file, &panel, &out),
        Cmd::Patch { src, dst, config } => {
            config::PatchConfig::load(&config).and_then(|c| cmd_patch(&src, &dst, &c))
        }
        Cmd::Flash {
            upg,
            arm,
            trust_marker_crc,
            updater_dir,
        } => cmd_flash(&upg, arm, trust_marker_crc, updater_dir.as_deref()),
        Cmd::Isp { sub, updater_dir } => cmd_isp(&sub, updater_dir.as_deref()),
        Cmd::Dis {
            bank,
            entries,
            case,
            range,
        } => cmd_dis(&bank, &entries, &case, range.as_deref()),
        Cmd::Xref { file, kind, addr } => cmd_xref(&file, kind, &addr),
        Cmd::Ddc { sub } => cmd_ddc(&sub),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}
