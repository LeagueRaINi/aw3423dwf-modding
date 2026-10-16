//! Files commands.

use crate::upg::Upg;
use crate::{crc8, edid};

pub fn cmd_upg(path: &str) -> Result<(), String> {
    let u = Upg::load(path).map_err(|e| e.to_string())?;
    for b in u.blocks() {
        println!("{b}");
    }
    let errs = u.verify();
    println!(
        "verify: {}",
        if errs.is_empty() {
            "OK".into()
        } else {
            errs.join(", ")
        }
    );
    u.require_valid_checksums()
}

pub fn cmd_edid(path: &str) -> Result<(), String> {
    let u = Upg::load(path).map_err(|e| e.to_string())?;
    for (i, b) in u.blocks().iter().enumerate() {
        let img = u.image(b);
        for (o, n) in edid::find(img) {
            let e = &img[o..o + 128 * (n + 1)];
            let fast: Vec<String> = edid::timings(e)
                .iter()
                .filter(|t| t.hz() > 100.0)
                .map(|t| t.to_string())
                .collect();
            println!(
                "img{i} {o:#08x} DEL{:04X} ext={n} checksums={} {}",
                edid::product_id(e),
                if edid::checksums_ok(e) { "ok" } else { "BAD" },
                fast.join("; ")
            );
        }
    }
    Ok(())
}

pub fn cmd_crc(path: &str) -> Result<(), String> {
    let u = Upg::load(path).map_err(|e| e.to_string())?;
    // CRCs of the flash layout (signatures stripped, banks in flash order), comparable with `isp crc`.
    for b in u.blocks() {
        let crcs: Vec<String> = u
            .flash_view(&b)
            .chunks(65536)
            .map(|c| format!("{:02x}", crc8::crc8(c)))
            .collect();
        println!(
            "panel '{}' @ flash {:#08x}: {}",
            b.panel,
            b.flash_start,
            crcs.join(" ")
        );
    }
    Ok(())
}

/// Write the image for `panel` as it sits in flash (signatures stripped, banks in flash order).
pub fn cmd_flashview(path: &str, panel: &str, out: &str) -> Result<(), String> {
    let u = Upg::load(path).map_err(|e| e.to_string())?;
    let b = u
        .blocks()
        .into_iter()
        .find(|b| panel == b.panel.to_string())
        .ok_or(format!("no image for panel '{panel}'"))?;
    let view = u.flash_view(&b);
    std::fs::write(out, &view).map_err(|e| e.to_string())?;
    println!(
        "panel '{}': {:#x} bytes, offset 0 = flash {:#08x} -> {out}",
        b.panel,
        view.len(),
        b.flash_start
    );
    Ok(())
}
