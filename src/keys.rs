//! OEM handshake keys, recovered at runtime from the user's copy of Dell's `Firmware Updater.exe`.
//!
//! The hub's rolling challenge-response needs one of Dell's 17 OEM keys. This crate does not ship
//! them: it stores only each key's length and SHA-256, and finds the matching byte runs in the
//! updater that sits next to `RtHIDlib.dll`. The order is the updater's probe order.

use hex_literal::hex;
use sha2::{Digest, Sha256};
use std::path::Path;

pub const UPDATER_EXE: &str = "Firmware Updater.exe";

#[rustfmt::skip]
const KEY_DIGESTS: [(usize, [u8; 32]); 17] = [
    (14, hex!("59d7740376f95bf026a563b8ea506493a7ad66828fb33e7235b1efa96c51ba91")),
    (16, hex!("73c16f1f37becbfd94e2a0a24c4942c4798fc78ca1e6ed70369d5a4559779faf")),
    (15, hex!("0a7d101488388dfa8e1434dec8e40cd26712c2fbd517333abe82bdd46e5cc4de")),
    (19, hex!("f24b55053ed0ed3113924749d79c62a857760419c93f9a232858f3c5a3470783")),
    (17, hex!("a10e943a23d04582890a501dccbc4a8515424f0240f672762f0d9c4826d2fb00")),
    (19, hex!("2e2de8388a8f961e6cb5c9266ed2e65988fc3b1aee7a722bf63ce82854fb38b9")),
    (15, hex!("1e111047d894ad43cccdc91c31ab6305a2263896189fdd636b2bed799aae16e7")),
    (18, hex!("042710adb008547516d442107bb67d8c33d547dab4faa8f75a409c27644bb004")),
    (15, hex!("129ea237fad47013e326ff1acdf5afaef7cd31025dad839343c1aac6a457d99f")),
    (18, hex!("0d253902545bfa0eec7c43711878bc7d3112ded67f8cd83311fd30e01c9ed465")),
    (12, hex!("6c835dfb0269db60f6e8d8a59555b2fe39df2590a39270aad92c8a844aff315f")),
    (17, hex!("f88caa8564f72b8ac2e45fedbe28a4186fc2fa0943fb39e3ab17eab90394b994")),
    (15, hex!("01727a94a133ea3b7bf08ae9f48c28859d1e6263a42977cf66db98249951ce12")),
    (17, hex!("874f051d20044b465a8c2db39cb0a4c66d0b7c4f3421326fdf883f581b82ab7f")),
    (16, hex!("7519a41f62cd8714cd4b056e7e804f225560d03f701b39844d7a0e64c5f72de4")),
    (14, hex!("b94a2c4a2ed8ef1b0dc826fab1cf55ef1dd18217fe36a2b607549a172cda2a61")),
    (15, hex!("279f63ad393aafee36662ccc463cd3c8b7b961a26152fde4666c278dfc5417cf")),
];

/// Read `Firmware Updater.exe` from `dir` and return the OEM keys in probe order.
pub fn load(dir: &Path) -> Result<Vec<Vec<u8>>, String> {
    let path = dir.join(UPDATER_EXE);
    let exe = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let keys = find(&exe);
    let missing: Vec<usize> = (0..keys.len()).filter(|&i| keys[i].is_none()).collect();
    if !missing.is_empty() {
        return Err(format!(
            "{} does not contain OEM keys {missing:?}; use the AW3423DWF M3B107 updater package",
            path.display()
        ));
    }
    Ok(keys.into_iter().flatten().collect())
}

/// Find each key by hashing every window of each distinct key length.
fn find(data: &[u8]) -> Vec<Option<Vec<u8>>> {
    let mut found = vec![None; KEY_DIGESTS.len()];
    let mut lengths: Vec<usize> = KEY_DIGESTS.iter().map(|&(n, _)| n).collect();
    lengths.sort_unstable();
    lengths.dedup();
    for n in lengths {
        for window in data.windows(n) {
            let digest: [u8; 32] = Sha256::digest(window).into();
            for (i, (len, want)) in KEY_DIGESTS.iter().enumerate() {
                if *len == n && *want == digest && found[i].is_none() {
                    found[i] = Some(window.to_vec());
                }
            }
        }
        if found.iter().all(Option::is_some) {
            break;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_keys_are_reported_by_index() {
        let dir = std::env::temp_dir().join("awtool-keys-test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(UPDATER_EXE), [0u8; 64]).unwrap();
        let err = load(&dir).unwrap_err();
        assert!(err.contains("[0, 1, 2"), "{err}");
    }

    #[test]
    #[ignore = "requires AWTOOL_UPDATER_DIR pointing at Dell's updater folder"]
    fn all_keys_are_found_in_dells_updater() {
        let dir = std::env::var("AWTOOL_UPDATER_DIR").expect("set AWTOOL_UPDATER_DIR");
        assert_eq!(load(Path::new(&dir)).unwrap().len(), KEY_DIGESTS.len());
    }
}
