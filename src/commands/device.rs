//! Device connection and flash-capacity decoding.

use crate::{isp, keys, rthid};
use std::path::{Path, PathBuf};

/// Open the monitor's hub through Dell's `RtHIDlib.dll`, authenticating with the OEM keys read from
/// the `Firmware Updater.exe` in the same folder.
pub(super) fn open_hub(updater_dir: Option<&Path>) -> Result<isp::Hub, String> {
    let dir = find_updater_dir(updater_dir)?;
    let keys = keys::load(&dir)?;
    let hid = rthid::RtHid::load(&dir.to_string_lossy())?;
    isp::Hub::open(Box::new(hid), keys)
}

fn find_updater_dir(explicit: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(d) = explicit {
        // LoadLibraryW does not resolve relative directories such as `..`.
        return std::path::absolute(d).map_err(|e| format!("{}: {e}", d.display()));
    }
    let mut dir = std::env::current_dir().map_err(|e| e.to_string())?;
    loop {
        if dir.join("RtHIDlib.dll").exists() && dir.join(keys::UPDATER_EXE).exists() {
            return Ok(dir);
        }
        if !dir.pop() {
            return Err(format!(
                "RtHIDlib.dll and {} not found in this or any parent directory; pass --updater-dir",
                keys::UPDATER_EXE
            ));
        }
    }
}

pub(super) fn jedec_size(id: [u8; 3]) -> Result<u32, String> {
    if !(0x10..=0x18).contains(&id[2]) || matches!(id[0], 0 | 0xFF) {
        return Err(format!(
            "unsupported JEDEC capacity/id {id:02x?}; cannot establish a 24-bit flash range"
        ));
    }
    Ok(1u32 << id[2])
}
