//! Panel output vertical-total floor (stock 1708 lines, 0x06AC).
//!
//! Addresses are bank-ordered flash-view offsets. Hardware bisection showed the output
//! pixel clock is limited near the stock D60E maximum (~1.022 GHz); above ~167 Hz the
//! firmware can only stay under it by shortening the vertical total below this floor.
//! Panel tolerance of the shorter vertical blank is not established.

use hex_literal::hex;

struct Site {
    name: &'static str,
    offset: usize,
    stock: &'static [u8],
    /// (high byte offset, low byte offset) pairs of the 1708 immediates.
    words: &'static [(usize, usize)],
}

const SITES: &[Site] = &[
    Site {
        name: "13:BFA4 high-refresh DV total clamp",
        offset: 0x13BFA4,
        stock: &hex!("C3EF94ACEE9406500A90CA097406F0A374AC"),
        words: &[(6, 3), (13, 17)],
    },
    Site {
        name: "14:C06C DV total floor test",
        offset: 0x14C06C,
        stock: &hex!("C3E53294ACE531940622"),
        words: &[(8, 4)],
    },
    Site {
        name: "14:C282 high-refresh DV total",
        offset: 0x14C282,
        stock: &hex!("7535067536AC"),
        words: &[(2, 5)],
    },
    Site {
        name: "14:D2CD clock-window DV total clamp",
        offset: 0x14D2CD,
        stock: &hex!("C394ACEE940650067E067FAC"),
        words: &[(5, 2), (9, 11)],
    },
    Site {
        name: "1B:69D9 DV total clamp",
        offset: 0x1B69D9,
        stock: &hex!("C394ACEE9406500A90C9B37406F0A374AC"),
        words: &[(5, 2), (12, 16)],
    },
    Site {
        name: "1B:77F2 DV total floor test",
        offset: 0x1B77F2,
        stock: &hex!("D3EF94ACEE94064003"),
        words: &[(6, 3)],
    },
];

/// Smallest accepted floor: 1440 active lines plus a token blank. Not a panel limit.
pub const DV_TOTAL_MIN_LOWEST: u16 = 1456;
pub const DV_TOTAL_MIN_STOCK: u16 = 1708;

/// Replace the 1708-line output vertical-total floor at every traced site as one transaction.
pub fn patch_dv_total_min(img: &mut [u8], lines: u16) -> Result<usize, String> {
    if !(DV_TOTAL_MIN_LOWEST..=DV_TOTAL_MIN_STOCK).contains(&lines) {
        return Err(format!(
            "panel.dv_total_min must be in {DV_TOTAL_MIN_LOWEST}..={DV_TOTAL_MIN_STOCK}"
        ));
    }
    for site in SITES {
        if img.get(site.offset..site.offset + site.stock.len()) != Some(site.stock) {
            return Err(format!(
                "{} at flash-view {:#08x}: expected stock M3B107 bytes; refusing partial/repeated patch",
                site.name, site.offset
            ));
        }
    }
    let [hi, lo] = lines.to_be_bytes();
    for site in SITES {
        let bytes = &mut img[site.offset..site.offset + site.stock.len()];
        for &(h, l) in site.words {
            bytes[h] = hi;
            bytes[l] = lo;
        }
    }
    Ok(SITES.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stock_sites() -> Vec<u8> {
        let mut img = vec![0; 0x1C0000];
        for site in SITES {
            img[site.offset..site.offset + site.stock.len()].copy_from_slice(site.stock);
        }
        img
    }

    #[test]
    fn every_word_is_the_stock_floor() {
        for site in SITES {
            for &(h, l) in site.words {
                assert_eq!(
                    [site.stock[h], site.stock[l]],
                    [0x06, 0xAC],
                    "{}",
                    site.name
                );
            }
        }
    }

    #[test]
    fn wrong_site_range_or_repeat_is_an_atomic_error() {
        for site in SITES {
            let mut img = stock_sites();
            img[site.offset] ^= 1;
            let original = img.clone();
            assert!(patch_dv_total_min(&mut img, 1676).is_err());
            assert_eq!(img, original, "{}", site.name);
        }
        let mut img = stock_sites();
        let original = img.clone();
        assert!(patch_dv_total_min(&mut img, 1709).is_err());
        assert!(patch_dv_total_min(&mut img, 1455).is_err());
        assert_eq!(img, original);
        assert_eq!(patch_dv_total_min(&mut img, 1676).unwrap(), 6);
        assert_eq!(&img[0x14C282..0x14C288], &hex!("75350675368C"));
        let patched = img.clone();
        assert!(patch_dv_total_min(&mut img, 1676).is_err());
        assert_eq!(img, patched);
    }
}
