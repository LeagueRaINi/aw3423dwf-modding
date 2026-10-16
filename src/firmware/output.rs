//! Output policy: the panel-output refresh descriptor, its clock window and the nominal timing getter.
//!
//! Addresses are bank-ordered flash-view offsets, not UPG file offsets. Verify
//! instructions as well as constants at each exact address before changing any
//! bytes. Inline clock data follows LCALL 3CBE/3CEF; it is big-endian u32 data.

use crate::config::OutputPatch;
use hex_literal::hex;

struct Site {
    name: &'static str,
    offset: usize,
    stock: &'static [u8],
    encoding: Encoding,
}

enum Encoding {
    Maximum { high: usize, low: usize },
    Nominal { high: usize, low: usize },
    Clock { at: usize, kind: Clock },
}

enum Clock {
    Nominal,
    Maximum,
    Minimum,
}

const SITES: &[Site] = &[
    Site {
        name: "runtime classifier-2 output maximum",
        offset: 0x009263,
        stock: &hex!("900001740675F072123A11"),
        encoding: Encoding::Maximum { high: 4, low: 7 },
    },
    Site {
        name: "startup output maximum",
        offset: 0x00E42E,
        stock: &hex!("90D60EE054FEF04402F0A37406F0A37472F0"),
        encoding: Encoding::Maximum { high: 12, low: 16 },
    },
    Site {
        name: "parameter 0x17 nominal/fallback refresh",
        offset: 0x19E491,
        stock: &hex!("122C47EF70057E067F72227E027F5822"),
        encoding: Encoding::Nominal { high: 7, low: 9 },
    },
    Site {
        name: "runtime nominal output clock",
        offset: 0x00927F,
        stock: &hex!("900007123CEF000F80C0"),
        encoding: Encoding::Clock {
            at: 6,
            kind: Clock::Nominal,
        },
    },
    Site {
        name: "runtime maximum output clock",
        offset: 0x009289,
        stock: &hex!("90000B123CEF000F9740"),
        encoding: Encoding::Clock {
            at: 6,
            kind: Clock::Maximum,
        },
    },
    Site {
        name: "runtime minimum output clock",
        offset: 0x009293,
        stock: &hex!("90000F123CEF000F6A1D"),
        encoding: Encoding::Clock {
            at: 6,
            kind: Clock::Minimum,
        },
    },
    Site {
        name: "startup nominal output clock",
        offset: 0x00E455,
        stock: &hex!("A3123CBE000F80C0"),
        encoding: Encoding::Clock {
            at: 4,
            kind: Clock::Nominal,
        },
    },
    Site {
        name: "startup maximum output clock",
        offset: 0x00E45D,
        stock: &hex!("90D619123CBE000F9740"),
        encoding: Encoding::Clock {
            at: 6,
            kind: Clock::Maximum,
        },
    },
    Site {
        name: "startup minimum output clock",
        offset: 0x00E467,
        stock: &hex!("90D61D123CBE000F6A1D"),
        encoding: Encoding::Clock {
            at: 6,
            kind: Clock::Minimum,
        },
    },
];

/// Patch the nine remaining traced output sites as one transaction.
/// `cap_hz` must also be applied to the seven existing refresh anchors by the caller.
/// Nominal refresh changes fixed/fallback paths too; this does not alter the 60 Hz
/// alternate getter, minimum refresh, timing totals/floors, policy flags or PLL code.
pub fn patch_output_policy(
    img: &mut [u8],
    cap_hz: u16,
    output: &OutputPatch,
) -> Result<usize, String> {
    output.validate(cap_hz)?;
    for site in SITES {
        if img.get(site.offset..site.offset + site.stock.len()) != Some(site.stock) {
            return Err(format!(
                "{} at flash-view {:#08x}: expected stock M3B107 instructions/constants; refusing partial/repeated output patch",
                site.name, site.offset
            ));
        }
    }
    // Every target was checked before the first write. Only immediate/data bytes change.
    for site in SITES {
        let bytes = &mut img[site.offset..site.offset + site.stock.len()];
        match &site.encoding {
            Encoding::Maximum { high, low } | Encoding::Nominal { high, low } => {
                let hz = match site.encoding {
                    Encoding::Maximum { .. } => cap_hz,
                    _ => output.nominal_hz,
                };
                let value = (hz * 10).to_be_bytes();
                bytes[*high] = value[0];
                bytes[*low] = value[1];
            }
            Encoding::Clock { at, kind } => {
                let value = match kind {
                    Clock::Nominal => output.clock_nominal_khz,
                    Clock::Maximum => output.clock_max_khz,
                    Clock::Minimum => output.clock_min_khz,
                };
                bytes[*at..*at + 4].copy_from_slice(&value.to_be_bytes());
            }
        }
    }
    Ok(SITES.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stock_sites() -> Vec<u8> {
        let mut img = vec![0; 0x1A0000];
        for site in SITES {
            img[site.offset..site.offset + site.stock.len()].copy_from_slice(site.stock);
        }
        img
    }

    fn policy() -> OutputPatch {
        OutputPatch {
            nominal_hz: 175,
            clock_nominal_khz: 1_077_576,
            clock_min_khz: 1_010_205,
            clock_max_khz: 1_083_685,
        }
    }

    #[test]
    fn any_wrong_site_or_truncated_image_is_an_atomic_error() {
        for site in SITES {
            let mut img = stock_sites();
            img[site.offset] ^= 1;
            let original = img.clone();
            assert!(patch_output_policy(&mut img, 175, &policy()).is_err());
            assert_eq!(img, original, "{}", site.name);
        }
        let mut img = stock_sites();
        img.truncate(0x19E49A);
        let original = img.clone();
        assert!(patch_output_policy(&mut img, 175, &policy()).is_err());
        assert_eq!(img, original);
    }

    #[test]
    fn invalid_policy_or_repeated_patch_is_an_atomic_error() {
        let mut img = stock_sites();
        let original = img.clone();
        assert!(patch_output_policy(&mut img, 6554, &policy()).is_err());
        assert_eq!(img, original);
        assert_eq!(patch_output_policy(&mut img, 175, &policy()).unwrap(), 9);
        let original = img.clone();
        assert!(patch_output_policy(&mut img, 175, &policy()).is_err());
        assert_eq!(img, original);
    }
}
