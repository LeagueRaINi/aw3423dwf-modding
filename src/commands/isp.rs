//! Read-only ISP commands and resumable dump handling.

use super::{
    device::{jedec_size, open_hub},
    parse_num,
};
use crate::cli::IspCmd;
use crate::{crc8, isp, upg::Upg};
use std::io::Seek;
use std::path::Path;

pub fn cmd_isp(sub: &IspCmd, updater_dir: Option<&Path>) -> Result<(), String> {
    let hub = open_hub(updater_dir)?;
    // Recovery: reboot the scaler without touching DDC (which is dead while the MCU is halted in ISP).
    if let IspCmd::Reboot = sub {
        hub.reboot();
        hub.close();
        println!(
            "reboot command sent; the screen blinks once and the OSD/buttons come back in a few seconds"
        );
        return Ok(());
    }
    let panel = hub.panel_id()?;
    println!("firmware version: {}", hub.version()?);
    println!("panel id: {:#04x} ('{}')", panel, panel as char);
    if let IspCmd::Info = sub {
        return Ok(());
    }
    eprintln!(
        "entering ISP: picture stays up, but the OSD and buttons are frozen until the scaler reboots"
    );
    hub.enter_isp()?;

    let id = hub.jedec_id()?;
    let size = jedec_size(id)?;
    println!(
        "flash JEDEC id {:02X} {:02X} {:02X} ({} MB)",
        id[0],
        id[1],
        id[2],
        size >> 20
    );
    match sub {
        IspCmd::Crc => {
            for row in (0..size).step_by(16 << 16) {
                let count = ((size - row) >> 16).min(16);
                let crcs =
                    (0..count).map(|k| hub.crc(row + (k << 16), 65536).map(|c| format!("{c:02x}")));
                println!(
                    "crc {row:06X}: {}",
                    crcs.collect::<Result<Vec<_>, _>>()?.join(" ")
                );
            }
            Ok(())
        }
        // Resumable, per-4KB CRC-checked dump to a file. Re-run the same command to continue
        // after an interruption; each 4 KB chunk is verified against the hardware CRC before it is kept.
        IspCmd::Dump { addr, len, out } => {
            use std::io::Write;
            let (addr, len) = (parse_num(addr)?, parse_num(len)?);
            validate_read_range(addr, len, size)?;
            const CHUNK: u32 = 0x1000;
            match std::fs::metadata(out) {
                Ok(metadata) if metadata.len() > u64::from(len) => {
                    return Err("existing dump is larger than the requested range".into());
                }
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
                _ => {}
            }
            let existing = match std::fs::read(out) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                Err(e) => return Err(e.to_string()),
            };
            if existing.len() > len as usize {
                return Err("existing dump is larger than the requested range".into());
            }
            let done0 = if existing.len() == len as usize {
                len
            } else {
                existing.len() as u32 / CHUNK * CHUNK
            };
            // Revalidate saved chunks against this address range/device before keeping them.
            for (i, chunk) in existing[..done0 as usize]
                .chunks(CHUNK as usize)
                .enumerate()
            {
                if hub.crc(addr + i as u32 * CHUNK, chunk.len() as u32)? != crc8::crc8(chunk) {
                    return Err(format!(
                        "saved dump chunk {i} does not match this device/range; output left untouched"
                    ));
                }
            }
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(out)
                .map_err(|e| e.to_string())?;
            f.set_len(done0 as u64).map_err(|e| e.to_string())?; // drop any partial tail
            f.seek(std::io::SeekFrom::End(0))
                .map_err(|e| e.to_string())?;
            if done0 > 0 {
                eprintln!(
                    "resuming at {:#08x} ({done0}/{len} bytes already done)",
                    addr + done0
                );
            }
            let t = std::time::Instant::now();
            let mut off = done0;
            while off < len {
                let n = CHUNK.min(len - off);
                match hub.read_flash(addr + off, n, &mut |_| {}) {
                    Ok(chunk) => {
                        f.write_all(&chunk)
                            .and_then(|_| f.flush())
                            .map_err(|e| e.to_string())?;
                        off += n;
                        let bps = off.saturating_sub(done0) as f64 / t.elapsed().as_secs_f64();
                        eprint!(
                            "\r{off}/{len} bytes, {bps:.0} B/s, eta {:.0}s   ",
                            (len - off) as f64 / bps.max(1.0)
                        );
                    }
                    Err(e) => {
                        eprintln!();
                        return Err(format!(
                            "dump stalled at {:#08x}: {e}. {off} bytes saved; re-run the same command to resume (after `isp reboot` if the OSD is unresponsive).",
                            addr + off
                        ));
                    }
                }
            }
            eprintln!();
            println!(
                "dumped {len} bytes @ {addr:#08x} -> {out} (every chunk verified against hardware CRC)"
            );
            Ok(())
        }
        IspCmd::Read { addr, len, out } => {
            let (addr, len) = (parse_num(addr)?, parse_num(len)?);
            validate_read_range(addr, len, size)?;
            let t = std::time::Instant::now();
            let data = hub.read_flash(addr, len, &mut |n| {
                if n % 3072 < 3 {
                    eprint!(
                        "\r{n}/{len} bytes, {:.0} B/s   ",
                        n as f64 / t.elapsed().as_secs_f64()
                    );
                }
            })?;
            std::fs::write(out, &data).map_err(|e| e.to_string())?;
            eprintln!();
            println!("read {len} bytes @ {addr:#08x} -> {out} (verified against hardware CRC)");
            Ok(())
        }
        // CRC equality is not monotonic over prefixes. Compare bytes in address order.
        IspCmd::Diff { upg, skip } => {
            let upg = Upg::load(upg).map_err(|e| e.to_string())?;
            upg.require_valid_checksums()?;
            let skip = parse_num(skip)? as usize;
            let b = upg
                .blocks()
                .into_iter()
                .find(|b| b.panel == panel as char)
                .ok_or("no image for this panel")?;
            let view = upg.flash_view(&b);
            let want = view
                .get(skip..)
                .ok_or("SKIP is outside the flash-view image")?;
            if want.is_empty() {
                return Err("SKIP leaves no bytes to compare".into());
            }
            let base = b
                .flash_start
                .checked_add(skip as u32)
                .ok_or("flash address overflow")?;
            validate_read_range(base, want.len() as u32, size)?;
            eprintln!("comparing bytes in order; ISP reads are slow (SKIP is a flash-view offset)");
            for (i, chunk) in want.chunks(0x1000).enumerate() {
                let off = i * 0x1000;
                let got = hub.read_flash(base + off as u32, chunk.len() as u32, &mut |_| {})?;
                if let Some(at) = got.iter().zip(chunk).position(|(have, want)| have != want) {
                    println!(
                        "first difference at flash {:#08x} = flash-view offset {:#x}: flash {:02x}, image {:02x}",
                        base + (off + at) as u32,
                        skip + off + at,
                        got[at],
                        chunk[at]
                    );
                    return Ok(());
                }
                eprint!("\rcompared {}/{} bytes   ", off + chunk.len(), want.len());
            }
            eprintln!();
            println!(
                "flash {base:#08x}+{:#x} matches flash-view[{skip:#x}..] byte for byte",
                want.len()
            );
            Ok(())
        }
        IspCmd::Info | IspCmd::Reboot => unreachable!("handled before ISP entry"),
    }
}

fn validate_read_range(start: u32, len: u32, size: u32) -> Result<(), String> {
    if isp::flash_range_end(start, len)? > size {
        return Err("requested range exceeds detected flash capacity".into());
    }
    Ok(())
}
