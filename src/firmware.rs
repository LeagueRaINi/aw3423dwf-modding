//! Reverse-engineered firmware code-site patches (M3B107, RTD2717Q 8051).
//!
//! These byte anchors are firmware *structure*, not user config — they encode where the refresh limits
//! live in the scaler code. Each is matched exactly (verifying the stock bytes) before being rewritten,
//! so a firmware revision with different code fails loudly instead of being mis-patched.

use hex_literal::hex;

mod output;
mod panel;
pub use output::patch_output_policy;
pub use panel::patch_dv_total_min;

/// Find every occurrence of `anchor` in `img`, verify each named byte's `old` value, then set it to `new`.
/// Returns the number of sites patched; errors on any old-value mismatch (wrong image / revision).
pub fn patch_anchor(
    img: &mut [u8],
    anchor: &[u8],
    edits: &[(usize, u8, u8)],
) -> Result<usize, String> {
    if anchor.is_empty() || edits.iter().any(|&(rel, _, _)| rel >= anchor.len()) {
        return Err("empty anchor or edit outside anchor".into());
    }
    // Validate the complete edit set first; an error must not leave a partial mutation.
    for (i, bytes) in img
        .windows(anchor.len())
        .enumerate()
        .filter(|(_, bytes)| *bytes == anchor)
    {
        for &(rel, old, _) in edits {
            if bytes[rel] != old {
                return Err(format!(
                    "anchor at {i:#x}: byte +{rel} is {:02x}, expected {old:02x}",
                    bytes[rel]
                ));
            }
        }
    }
    let mut n = 0;
    let mut i = 0;
    while i + anchor.len() <= img.len() {
        if &img[i..i + anchor.len()] == anchor {
            for &(rel, old, new) in edits {
                if img[i + rel] != old {
                    return Err(format!(
                        "anchor at {i:#x}: byte +{rel} is {:02x}, expected {old:02x}",
                        img[i + rel]
                    ));
                }
                img[i + rel] = new;
            }
            n += 1;
            i += anchor.len();
        } else {
            i += 1;
        }
    }
    Ok(n)
}

/// Raise the firmware refresh ceiling to `hz` whole Hz in a full panel flash-view image:
///   - the two DP upper clamps `min(v, 0x0672=165.0)` over XDATA 0xCA07:0xCA08 and 0xCA21:0xCA22,
///   - the two input-rate validators that reject `v > 0x06A4 = 170.0`,
///   - the "is refresh > 165?" query over 0xD24D:0xD24E,
///   - the panel-output ceiling: the block-6 stores of 165.0 into C9A7:C9A8, copied to the cap
///     D5D3:D5D4 that limits D24D:D24E.
///
/// These values are 0.1 Hz. The 285 kHz input horizontal-rate validator (not raised)
/// caps input at ~192 Hz with 1484-line timings. Separate D60E policy and clock-window
/// constraints are handled separately by `patch_output_policy` when explicitly requested.
pub fn raise_refresh_ceiling(img: &mut [u8], hz: u16) -> Result<usize, String> {
    let v = hz
        .checked_mul(10)
        .filter(|&v| v != 0)
        .ok_or("refresh ceiling does not fit u16 tenths of Hz")?;
    let (lo, hi) = ((v & 0xFF) as u8, (v >> 8) as u8);
    let mut patched = img.to_vec();
    let mut n = 0;
    let apply = |img: &mut [u8],
                 name: &str,
                 anchor: &[u8],
                 edits: &[(usize, u8, u8)]|
     -> Result<usize, String> {
        let count = img.windows(anchor.len()).filter(|w| *w == anchor).count();
        if count != 1 {
            return Err(format!(
                "{name}: expected exactly one M3B107 anchor, found {count}; refusing partial/repeated patch"
            ));
        }
        patch_anchor(img, anchor, edits)
    };
    // DP output clamps: cmp lo@1, cmp hi@7, force hi@11, force lo@15.
    for anchor in [
        hex!("947290CA07E0940640077406F0A37472"),
        hex!("947290CA21E0940640077406F0A37472"),
    ] {
        n += apply(
            &mut patched,
            "DP output clamp",
            &anchor,
            &[(1, 0x72, lo), (7, 0x06, hi), (11, 0x06, hi), (15, 0x72, lo)],
        )?;
    }
    // Input-rate validators (reject > 0x06A4; SETB C supplies the extra borrow): cmp lo@3, cmp hi@6.
    for anchor in [hex!("D3ED94A4EC94065017"), hex!("D3EF94A4EE94064006")] {
        n += apply(
            &mut patched,
            "input-rate validator",
            &anchor,
            &[(3, 0xA4, lo), (6, 0x06, hi)],
        )?;
    }
    // "refresh > 165?" query over D24D:D24E: cmp lo@1, cmp hi@7.
    n += apply(
        &mut patched,
        "D24D refresh query",
        &hex!("947290D24DE09406"),
        &[(1, 0x72, lo), (7, 0x06, hi)],
    )?;
    // Panel-output ceiling: C9A7:C9A8 stores -> cap D5D3:D5D4. Two cases.
    n += apply(
        &mut patched,
        "C9A6 cap store",
        &hex!("90C9A6EFF0A37406F07472"),
        &[(7, 0x06, hi), (10, 0x72, lo)],
    )?;
    n += apply(
        &mut patched,
        "C9A7 cap store",
        &hex!("90C9A77406F0A37472"),
        &[(4, 0x06, hi), (8, 0x72, lo)],
    )?;
    img.copy_from_slice(&patched);
    Ok(n)
}

/// Monitor-name descriptor anchor: `00 00 00 FC 00 "AW3423DWF"` (the field continues with `\n` + spaces).
pub const NAME_DESCRIPTOR_PREFIX: [u8; 5] = hex!("000000FC00");

/// Display-range-limits descriptor anchor (flags 0x0C, min 48, max 165); max-vertical byte is at +6.
pub const RANGE_DESCRIPTOR_ANCHOR: [u8; 7] = hex!("000000FD0C30A5");
pub const RANGE_MAXV_OFFSET: usize = 6;
pub const RANGE_MAXV_OLD: u8 = 0xA5;

/// Raise the two statically traced input horizontal-rate checks (0.1 kHz units).
/// Does not change physical input bandwidth or any output timing registers.
pub fn patch_input_horizontal(img: &mut [u8], khz: u16) -> Result<usize, String> {
    if !(285..=300).contains(&khz) {
        return Err("input horizontal maximum must be in 285..=300 kHz".into());
    }
    let sites: &[(usize, &[u8])] = &[
        (0x00A354, &hex!("D3ED9422EC940B5031")),
        (0x00A39E, &hex!("D3EF9422EE940B5017")),
    ];
    for &(at, stock) in sites {
        if img.get(at..at + stock.len()) != Some(stock) {
            return Err(format!(
                "input horizontal validator at {at:#x}: expected stock M3B107 instructions"
            ));
        }
    }
    let [hi, lo] = (khz * 10).to_be_bytes();
    for &(at, _) in sites {
        img[at + 3] = lo;
        img[at + 6] = hi;
    }
    Ok(sites.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_horizontal_checks_are_exact_and_atomic() {
        let mut img = vec![0; 0xA400];
        img[0xA354..0xA35D].copy_from_slice(&hex!("D3ED9422EC940B5031"));
        let before = img.clone();
        assert!(patch_input_horizontal(&mut img, 300).is_err());
        assert_eq!(img, before);
        img[0xA39E..0xA3A7].copy_from_slice(&hex!("D3EF9422EE940B5017"));
        assert_eq!(patch_input_horizontal(&mut img, 300).unwrap(), 2);
        assert_eq!(&img[0xA354..0xA35D], &hex!("D3ED94B8EC940B5031"));
        assert_eq!(&img[0xA39E..0xA3A7], &hex!("D3EF94B8EE940B5017"));
        let before = img.clone();
        assert!(patch_input_horizontal(&mut img, 300).is_err());
        assert_eq!(img, before);
    }

    #[test]
    fn malformed_or_mismatched_edits_leave_image_unchanged() {
        let original = vec![1, 2, 1, 2];
        for (anchor, edits) in [
            (vec![], vec![]),
            (vec![1, 2], vec![(2, 0, 3)]),
            (vec![1, 2], vec![(0, 1, 9), (1, 7, 9)]),
        ] {
            let mut img = original.clone();
            assert!(patch_anchor(&mut img, &anchor, &edits).is_err());
            assert_eq!(img, original);
        }
    }

    #[test]
    fn missing_refresh_anchor_is_an_atomic_error() {
        let mut img = hex!("947290CA07E0940640077406F0A37472").to_vec();
        let original = img.clone();
        assert!(raise_refresh_ceiling(&mut img, 240).is_err());
        assert_eq!(img, original);
        assert!(raise_refresh_ceiling(&mut img, 6554).is_err());
    }
}
