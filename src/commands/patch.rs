//! Patch commands.

use crate::upg::Upg;
use crate::{config, edid, firmware};
use hex_literal::hex;
use sha2::{Digest, Sha256};

/// SHA-256 of Dell's stock `AW3423DWF_M3B107_20240130(ISP).UPG`, the input every profile targets.
pub const STOCK_M3B107_SHA256: [u8; 32] =
    hex!("96b95afbefacacedb9d77f25fe142755088d3783e64c5a2deda40ec9c8ce7808");

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Overwrite the 13-byte monitor-name field in every matching name descriptor with `base+suffix`.
fn patch_name(e: &mut [u8], base: &str, suffix: &str) -> Result<usize, String> {
    let mut field = format!("{base}{suffix}").into_bytes();
    if field.len() > 12 {
        return Err(format!(
            "monitor name '{base}{suffix}' is too long (max 12 chars)"
        ));
    }
    field.push(0x0A); // descriptor-string terminator
    field.resize(13, 0x20);
    let mut anchor = firmware::NAME_DESCRIPTOR_PREFIX.to_vec();
    anchor.extend_from_slice(base.as_bytes());
    let mut n = 0;
    for i in [54, 72, 90, 108] {
        if e.get(i..i + anchor.len()) == Some(anchor.as_slice()) {
            e[i + 5..i + 18].copy_from_slice(&field);
            n += 1;
        }
    }
    Ok(n)
}

/// Apply a declarative [`PatchConfig`] to a UPG and fix every checksum.
pub fn cmd_patch(src: &str, dst: &str, config: &config::PatchConfig) -> Result<(), String> {
    let mut u = Upg::load(src).map_err(|e| e.to_string())?;
    if Sha256::digest(u.bytes())[..] != STOCK_M3B107_SHA256 {
        eprintln!(
            "warning: {src} is not Dell's stock M3B107 UPG (sha256 {}); profiles are written against stock",
            sha256_hex(u.bytes())
        );
    }
    let changes = apply_patch(&mut u, config)?;
    u.save(dst).map_err(|e| e.to_string())?;
    println!(
        "{changes} patches applied, wrote {dst}; container checksums OK (signatures not regenerated)"
    );
    println!("sha256 {}", sha256_hex(u.bytes()));
    Ok(())
}

/// Build all modified panel images before committing them to the container.
/// On failure neither the input container nor an output file is modified.
pub fn apply_patch(u: &mut Upg, config: &config::PatchConfig) -> Result<usize, String> {
    config.validate()?;
    u.require_valid_checksums()?;
    if !config.mode.is_empty() {
        let labels: Vec<&str> = config.mode.iter().map(|m| m.label.as_str()).collect();
        println!("DisplayID modes: {}", labels.join(", "));
    }
    if let Some(output) = &config.output {
        println!(
            "Experimental output policy: maximum {} Hz, nominal/fallback {} Hz, clock window {}..={} kHz (nominal {} kHz); hardware behavior unvalidated",
            config.refresh.as_ref().unwrap().cap_hz,
            output.nominal_hz,
            output.clock_min_khz,
            output.clock_max_khz,
            output.clock_nominal_khz
        );
    }
    if let Some(panel) = &config.panel {
        println!(
            "Experimental panel DV total floor: {} lines (stock 1708); panel tolerance unvalidated",
            panel.dv_total_min
        );
    }
    let mut changes = 0;
    let (mut pids, mut names, mut ranges, mut modes) = (0, 0, 0, 0);
    let mut images = Vec::new();
    for b in u.blocks() {
        let mut freesync_blocks = 0;
        let previous = (pids, names, ranges, modes);
        if b.model != "AW3423DWF" || b.version != "M3B107" || b.chip != "RTD2717Q-VDL-CG" {
            return Err(format!(
                "unsupported patch target: {b}; only the analysed M3B107 profile is supported"
            ));
        }
        let mut img = u.flash_view(&b);
        for (o, n) in edid::find(&img) {
            let e = &mut img[o..o + 128 * (n + 1)];
            if !edid::checksums_ok(e) {
                return Err(format!(
                    "panel '{}' EDID at {o:#x}: invalid checksums",
                    b.panel
                ));
            }
            if let Some(p) = &config.pid
                && let Some(new) = p.remap(edid::product_id(e))
            {
                e[10..12].copy_from_slice(&new.to_le_bytes());
                pids += 1;
            }
            if let Some(name) = &config.name {
                names += patch_name(e, &name.base, &name.suffix)?;
            }
            if let Some(f) = &config.freesync {
                freesync_blocks += edid::patch_dp_freesync(e, f.dp_max_hz)?;
            }
            if let Some(r) = &config.range {
                for at in [54, 72, 90, 108] {
                    let count = firmware::patch_anchor(
                        &mut e[at..at + 18],
                        &firmware::RANGE_DESCRIPTOR_ANCHOR,
                        &[(
                            firmware::RANGE_MAXV_OFFSET,
                            firmware::RANGE_MAXV_OLD,
                            r.max_vhz,
                        )],
                    )?;
                    ranges += count;
                    if count != 0 {
                        if let Some(h) = r.max_h_khz {
                            // EDID 1.4 range offset flag bit 3 extends maximum H by 255.
                            e[at + 4] = (e[at + 4] & !8) | if h > 255 { 8 } else { 0 };
                            e[at + 8] = if h > 255 { h - 255 } else { h } as u8;
                        }
                        if let Some(clock) = r.max_pclk_mhz {
                            e[at + 9] = (clock / 10) as u8;
                        }
                    }
                }
            }
            // Preserve the native slot's flags/polarities, but encode each mode's own geometry.
            if !config.mode.is_empty() {
                let mut slots: Vec<usize> = edid::timings(e)
                    .into_iter()
                    .filter(|t| t.kind == edid::Kind::DisplayId1)
                    .map(|t| t.at)
                    .collect();
                slots.sort_unstable();
                if !slots.is_empty() && slots.len() != config.mode.len() {
                    return Err(format!(
                        "panel '{}' EDID {o:#x}: {} DisplayID slots but {} modes requested",
                        b.panel,
                        slots.len(),
                        config.mode.len()
                    ));
                }
                if slots.len() == config.mode.len() {
                    let rslot = slots[slots.len() - 1];
                    let rbytes = e[rslot..rslot + 20].to_vec();
                    for (&slot, m) in slots.iter().zip(&config.mode) {
                        e[slot..slot + 20].copy_from_slice(&rbytes);
                        edid::write_did1(&mut e[slot..slot + 20], &mode_timing(m));
                    }
                    let bases: std::collections::BTreeSet<_> =
                        slots.iter().map(|s| s / 128 * 128).collect();
                    for base in bases {
                        edid::fix_displayid_checksum(e, base);
                    }
                    modes += config.mode.len();
                }
            }
            edid::fix_block_checksums(e);
        }
        if let Some(r) = &config.refresh {
            changes += firmware::raise_refresh_ceiling(&mut img, r.cap_hz)?;
            if let Some(output) = &config.output {
                changes += firmware::patch_output_policy(&mut img, r.cap_hz, output)?;
            }
        }
        if let Some(panel) = &config.panel {
            changes += firmware::patch_dv_total_min(&mut img, panel.dv_total_min)?;
        }
        if let Some(input) = &config.input {
            changes += firmware::patch_input_horizontal(&mut img, input.max_h_khz)?;
        }
        if config.freesync.is_some() {
            if freesync_blocks != 6 {
                return Err(format!(
                    "panel '{}': expected six DP AMD FreeSync blocks, found {freesync_blocks}",
                    b.panel
                ));
            }
            changes += freesync_blocks;
        }
        for (requested, count, name) in [
            (config.pid.is_some(), pids - previous.0, "PID"),
            (config.name.is_some(), names - previous.1, "name"),
            (config.range.is_some(), ranges - previous.2, "range"),
            (
                !config.mode.is_empty(),
                modes - previous.3,
                "DisplayID mode",
            ),
        ] {
            if requested && count == 0 {
                return Err(format!(
                    "panel '{}': no matching {name} targets; refusing incomplete patch",
                    b.panel
                ));
            }
        }
        images.push((b, img));
    }
    for (b, img) in images {
        u.replace_flash_view(&b, &img)?;
    }
    Ok(changes + pids + names + ranges + modes)
}

fn mode_timing(m: &config::Mode) -> edid::DidTiming {
    edid::DidTiming {
        ha: m.ha,
        hfp: m.hfp,
        hsw: m.hsw,
        hbl: m.hbl,
        va: m.va,
        vfp: m.vfp,
        vsw: m.vsw,
        vbl: m.vbl,
        pclk_khz: m.pclk_khz,
    }
}
