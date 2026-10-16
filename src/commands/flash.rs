//! Direct ISP update orchestration. Address/marker policy lives in crate::flash.

use super::device::{jedec_size, open_hub};
use crate::flash::{self, MARKER_ADDR};
use crate::{crc8, upg::Upg};
use std::path::Path;

/// Flash a patched UPG's panel-matched image to the monitor over direct ISP.
/// Dry-run by default (reads only, prints the plan). `--arm` performs writes in marker-first order.
/// Recovery depends on the reverse-engineered boot-selection behaviour; it is not a guarantee.
///   disable WP -> invalidate marker (also self-tests the program primitive) -> erase+write+verify each changed
///   block -> rewrite marker -> reboot.
pub fn cmd_flash(
    upg_path: &str,
    arm: bool,
    trust_marker_crc: bool,
    updater_dir: Option<&Path>,
) -> Result<(), String> {
    let upg = Upg::load(upg_path).map_err(|e| e.to_string())?;
    upg.require_valid_checksums()?;
    for block in upg.blocks() {
        flash::validate_target(&block, upg.flash_view(&block).len())?;
    }

    let hub = open_hub(updater_dir)?;
    let panel = hub.panel_id()?;
    let blk = upg
        .blocks()
        .into_iter()
        .find(|b| panel as char == b.panel)
        .ok_or("no image in this UPG for the monitor's panel")?;
    let view = upg.flash_view(&blk);
    let base = blk.flash_start;
    println!(
        "target: panel '{}', image {} -> flash {:#08x}, {:#x} bytes",
        blk.panel,
        blk.version,
        base,
        view.len()
    );
    hub.enter_isp()?;
    let id = hub.jedec_id()?;
    let size = jedec_size(id)?;
    if size < 0x800000 {
        return Err("flash is too small for the supported 8 MB layout".into());
    }

    // Compare each 64 KB block of the patched image against what the device currently holds.
    let mut changed = vec![];
    for (i, chunk) in view.chunks(0x10000).enumerate() {
        let block = (base >> 16) as u8 + i as u8;
        let want = crc8::crc8(chunk);
        let have = hub.crc((block as u32) << 16, chunk.len() as u32)?;
        if want != have {
            changed.push((block, i, chunk.len()));
        }
    }
    let mkblock = (MARKER_ADDR >> 16) as u8;
    let mkoff = (MARKER_ADDR & 0xFFFF) as usize;
    let crc_of = |edit: &dyn Fn(&mut [u8])| {
        let mut b = vec![0xFFu8; 0x10000];
        edit(&mut b);
        crc8::crc8(&b)
    };
    let crc_present = crc_of(&|b| {
        b[mkoff] = 0xAA;
        b[mkoff + 1] = 0x55;
    }); // 0x69
    let crc_invalid = crc_of(&|b| {
        b[mkoff] = 0x00;
        b[mkoff + 1] = 0x00;
    }); // 0xB7
    let marker = hub.crc(MARKER_ADDR & !0xFFFF, 0x10000)?;
    let marker_state = match marker {
        m if m == crc_present => "AA55 present",
        m if m == crc_invalid => "invalidated (prior interrupted flash)",
        _ => "UNEXPECTED",
    };
    println!(
        "complete-marker block {mkblock:#04x} crc {marker:02x} ({marker_state}); flash status reg1 {:02x}",
        hub.status_reg1()?
    );
    if changed.is_empty() {
        if !arm && marker == crc_present {
            println!(
                "firmware and marker CRCs match; no differences detected (CRC-8 collisions remain possible)."
            );
            return Ok(());
        }
        println!(
            "firmware CRCs match; checking the complete-marker before deciding whether to write."
        );
    } else {
        println!(
            "{} block(s) differ and would be erased+rewritten:",
            changed.len()
        );
        for (b, _, _) in &changed {
            println!("  block {b:#04x}  (flash {:#08x})", (*b as u32) << 16);
        }
    }
    if !arm {
        println!("\nDRY RUN — nothing was written. Re-run with --arm to flash.");
        println!(
            "Before flashing, set Windows to 165 Hz or lower: the monitor reboots into the new firmware with the\n\
             current mode, and a mode it rejects leaves you with a black screen."
        );
        return Ok(());
    }
    // Inspect all bytes before treating this block as disposable. An 8-bit CRC cannot establish that,
    // unless the user explicitly accepts the CRC for the normal AA55-present state.
    let marker_complete = if trust_marker_crc {
        if marker != crc_present {
            return Err(format!(
                "--trust-marker-crc requires the AA55-present marker CRC {crc_present:02x}, got {marker:02x}; re-run without it for a full byte check"
            ));
        }
        eprintln!("skipping marker-block byte read (--trust-marker-crc; CRC shows AA55 present)");
        true
    } else {
        eprintln!(
            "checking the entire 64 KB marker block before erase (ISP byte reads are slow) ..."
        );
        let marker_bytes = hub.read_flash(flash::MARKER_BLOCK, 0x10000, &mut |n| {
            if n % 3072 < 3 {
                eprint!("\rmarker validation: {n}/65536 bytes   ");
            }
        })?;
        eprintln!();
        flash::validate_marker_block(&marker_bytes)?
    };
    if changed.is_empty() && marker_complete {
        println!("no differences detected; nothing to write.");
        return Ok(());
    }

    // Destructive from here. Marker-first ordering follows the project's recovery model.
    println!("\narming: disabling flash write-protect + clearing status-register protect ...");
    hub.arm()?;

    // 1) Invalidate the marker (AA55 -> 0000, a 1->0 program needing no erase) so an interruption falls back.
    println!("invalidating complete-marker ...");
    // A previous interruption may have left self-test bytes. Clean only after byte validation.
    if !marker_complete {
        hub.erase_block(mkblock)?;
    }
    hub.write_byte(MARKER_ADDR, 0x00)?;
    hub.write_byte(MARKER_ADDR + 1, 0x00)?;
    if hub.crc(MARKER_ADDR & !0xFFFF, 0x10000)? != crc_invalid {
        return Err("marker invalidate did not verify; ABORTING before any firmware block was touched. Run `isp reboot`.".into());
    }

    // 2) Self-test the 64-byte page-program primitive on the throwaway marker block, at non-zero page
    //    offsets, BEFORE erasing any firmware. If 64-byte chunk programming misbehaves, this catches it here.
    let t1 = flash::self_test_pattern(0x5A);
    let t2 = flash::self_test_pattern(0xA5);
    hub.write_bytes(0x7F0040, &t1)?;
    hub.write_bytes(0x7F0080, &t2)?;
    let want = crc_of(&|b| {
        b[mkoff] = 0x00;
        b[mkoff + 1] = 0x00;
        b[0x40..0x40 + 64].copy_from_slice(&t1);
        b[0x80..0x80 + 64].copy_from_slice(&t2);
    });
    let got = hub.crc(MARKER_ADDR & !0xFFFF, 0x10000)?;
    if got != want {
        return Err(format!(
            "page-program self-test failed (crc {got:02x}, expected {want:02x}); the 64-byte write primitive is wrong. ABORTING before any firmware block was touched. Run `isp reboot`; the marker block is cleaned up on the next flash."
        ));
    }
    // 2b) Self-test the 4 KB sector-erase on the same throwaway block (erases the test bytes just written).
    hub.sector_erase(MARKER_ADDR & !0xFFFF)?; // sector 0 of the marker block; sector_erase blank-checks internally
    println!("  write + sector-erase primitives verified on the marker block; proceeding.");

    // 3) Erase + write + verify each changed firmware block.
    for (b, i, len) in &changed {
        let data = &view[i * 0x10000..i * 0x10000 + len];
        let t = std::time::Instant::now();
        hub.flash_block(*b, data, &mut |n| {
            if n % 4096 < 64 {
                eprint!(
                    "\rflashing block {b:#04x} ... {n}/{len} B, {:.0} B/s   ",
                    n as f64 / t.elapsed().as_secs_f64()
                );
            }
        })?;
        eprintln!("\rflashing block {b:#04x} ... done, verified ({len} B)        ");
    }

    // 4) Rewrite the marker LAST (Dell's path): erase its marker-only block, write AA then 55, verify 0x69.
    println!("restoring complete-marker ...");
    hub.erase_block(mkblock)?;
    hub.write_byte(MARKER_ADDR, 0xAA)?;
    hub.write_byte(MARKER_ADDR + 1, 0x55)?;
    let got = hub.crc(MARKER_ADDR & !0xFFFF, 0x10000)?;
    if got != crc_present {
        return Err(format!(
            "marker restore did not verify (crc {got:02x}, expected {crc_present:02x}); the image is written but NOT marked valid. Golden-image fallback is expected but not verified here. Re-run the flash to retry the marker."
        ));
    }
    println!(
        "flash complete; hardware CRC-8 checks passed (not cryptographic or byte-for-byte verification)."
    );
    Ok(())
}
