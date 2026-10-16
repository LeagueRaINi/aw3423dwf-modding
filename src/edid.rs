//! EDID templates embedded in the firmware images, and the patches we apply to them.

const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
pub const HBR3_PAYLOAD: f64 = 25.92e9;

/// (offset, extension count) of every EDID in `img`.
pub fn find(img: &[u8]) -> Vec<(usize, usize)> {
    img.windows(8)
        .enumerate()
        .filter(|(_, w)| *w == HEADER)
        .filter_map(|(o, _)| {
            let n = *img.get(o + 126)? as usize;
            (o + 128 * (n + 1) <= img.len()).then_some((o, n))
        })
        .collect()
}

pub fn product_id(e: &[u8]) -> u16 {
    u16::from_le_bytes([e[10], e[11]])
}

/// Patch both AMD version-3 maximum fields in stock DP EDIDs. HDMI is untouched.
/// Validate every target before editing; caller repairs EDID block checksums.
pub fn patch_dp_freesync(e: &mut [u8], maximum: u8) -> Result<usize, String> {
    if maximum < 48 || e.len() < 128 || !e.len().is_multiple_of(128) {
        return Err("invalid FreeSync maximum or EDID length".into());
    }
    // EDID 1.4 digital interface code 5 is DisplayPort.
    if e[18..20] != [1, 4] || e[20] & 0x8f != 0x85 {
        return Ok(0);
    }
    let mut targets = Vec::new();
    for base in (128..e.len()).step_by(128) {
        if e[base] != 2 || e[base + 2] == 0 {
            continue;
        }
        let end = e[base + 2] as usize;
        if !(4..=127).contains(&end) {
            return Err("invalid CTA data boundary".into());
        }
        let mut pos = 4;
        while pos < end {
            let header = e[base + pos];
            let size = (header & 31) as usize;
            let start = base + pos + 1;
            if pos + 1 + size > end {
                return Err("truncated CTA data block".into());
            }
            let data = &e[start..start + size];
            if header >> 5 == 3 && data.starts_with(&[0x1a, 0, 0]) {
                if size != 18
                    || data[3] != 3
                    || data[5] != 48
                    || data[6] != 165
                    || data[13] != 165
                    || data[14] & 3 != 0
                {
                    return Err("expected stock M3B107 DP AMD version-3 range 48..165; refusing mismatched/repeated patch".into());
                }
                targets.push(start);
            }
            pos += 1 + size;
        }
    }
    if targets.len() > 1 {
        return Err("duplicate AMD FreeSync blocks in DP EDID".into());
    }
    for &start in &targets {
        e[start + 6] = maximum;
        e[start + 13] = maximum;
        // u8 maximum has zero upper two bits; preserve all unrelated flags.
        e[start + 14] &= !3;
    }
    Ok(targets.len())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Dtd,
    DisplayId1,
}

#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub kind: Kind,
    /// offset from the start of the EDID
    pub at: usize,
    pub ha: u32,
    pub hb: u32,
    pub va: u32,
    pub vb: u32,
    pub pclk: u64,
}

impl Timing {
    pub fn hz(&self) -> f64 {
        self.pclk as f64 / ((self.ha + self.hb) as f64 * (self.va + self.vb) as f64)
    }

    pub fn fits_10bpc(&self) -> bool {
        // Payload-only estimate: does not establish link overhead, negotiated bpc or panel support.
        self.pclk as f64 * 30.0 <= HBR3_PAYLOAD
    }
}

impl std::fmt::Display for Timing {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "{:?}@{:#x} {}x{} total {}x{} {:.2} MHz {:.2} Hz 10bpc-payload={}",
            self.kind,
            self.at,
            self.ha,
            self.va,
            self.ha + self.hb,
            self.va + self.vb,
            self.pclk as f64 / 1e6,
            self.hz(),
            if self.fits_10bpc() { "ok" } else { "NO" }
        )
    }
}

fn dtd(e: &[u8], at: usize) -> Option<Timing> {
    let b = e.get(at..at + 18)?;
    let pclk = u16::from_le_bytes([b[0], b[1]]) as u64 * 10_000;
    (pclk != 0).then(|| Timing {
        kind: Kind::Dtd,
        at,
        ha: b[2] as u32 | ((b[4] as u32 >> 4) << 8),
        hb: b[3] as u32 | ((b[4] as u32 & 15) << 8),
        va: b[5] as u32 | ((b[7] as u32 >> 4) << 8),
        vb: b[6] as u32 | ((b[7] as u32 & 15) << 8),
        pclk,
    })
}

fn le16(b: &[u8], o: usize) -> u32 {
    u16::from_le_bytes([b[o], b[o + 1]]) as u32
}

/// All detailed timings in the base block, CTA-861 extensions and DisplayID Type I blocks.
pub fn timings(e: &[u8]) -> Vec<Timing> {
    let mut out: Vec<Timing> = (0..4).filter_map(|i| dtd(e, 54 + 18 * i)).collect();
    for blk in 1..e.len() / 128 {
        let base = 128 * blk;
        let c = &e[base..base + 128];
        match c[0] {
            0x02 => {
                let mut i = c[2] as usize;
                if !(4..=127).contains(&i) {
                    continue;
                } // zero means no CTA DTDs
                while i + 18 <= 127 {
                    out.extend(dtd(e, base + i));
                    i += 18;
                }
            }
            0x70 => {
                let end = 5 + c[2] as usize;
                if end >= 127 {
                    continue;
                } // leave room for section checksum and EDID checksum
                let mut p = 5;
                while p + 3 <= end {
                    let (tag, ln) = (c[p], c[p + 2] as usize);
                    if p + 3 + ln > end {
                        break;
                    }
                    if tag == 0 && ln == 0 {
                        break;
                    }
                    if tag == 0x03 && ln.is_multiple_of(20) {
                        for k in (0..ln).step_by(20) {
                            let at = base + p + 3 + k;
                            let t = &e[at..at + 20];
                            out.push(Timing {
                                kind: Kind::DisplayId1,
                                at,
                                ha: le16(t, 4) + 1,
                                hb: le16(t, 6) + 1,
                                va: le16(t, 12) + 1,
                                vb: le16(t, 14) + 1,
                                pclk: ((t[0] as u64 | (t[1] as u64) << 8 | (t[2] as u64) << 16)
                                    + 1)
                                    * 10_000,
                            });
                        }
                    }
                    p += 3 + ln;
                }
            }
            _ => {}
        }
    }
    out
}

pub struct DidTiming {
    pub ha: u16,
    pub hfp: u16,
    pub hsw: u16,
    pub hbl: u16,
    pub va: u16,
    pub vfp: u16,
    pub vsw: u16,
    pub vbl: u16,
    pub pclk_khz: u32,
}

/// Rewrite a DisplayID Type I timing in place, keeping its option byte and sync polarities.
pub fn write_did1(t: &mut [u8], v: &DidTiming) {
    let (hpol, vpol) = (t[9] & 0x80, t[17] & 0x80);
    t[0..3].copy_from_slice(&(v.pclk_khz / 10 - 1).to_le_bytes()[..3]);
    for (o, x) in [
        (4, v.ha),
        (6, v.hbl),
        (8, v.hfp),
        (10, v.hsw),
        (12, v.va),
        (14, v.vbl),
        (16, v.vfp),
        (18, v.vsw),
    ] {
        t[o..o + 2].copy_from_slice(&(x - 1).to_le_bytes());
    }
    t[9] |= hpol;
    t[17] |= vpol;
}

/// DisplayID section: 4-byte header at base+1, payload, one checksum byte.
pub fn fix_displayid_checksum(e: &mut [u8], base: usize) {
    let end = base + 5 + e[base + 2] as usize;
    let s = e[base + 1..end].iter().fold(0u8, |a, &b| a.wrapping_add(b));
    e[end] = s.wrapping_neg();
}

pub fn fix_block_checksums(e: &mut [u8]) {
    for blk in e.chunks_mut(128) {
        let s = blk[..127].iter().fold(0u8, |a, &b| a.wrapping_add(b));
        blk[127] = s.wrapping_neg();
    }
}

pub fn checksums_ok(e: &[u8]) -> bool {
    !e.is_empty()
        && e.len().is_multiple_of(128)
        && e.as_chunks::<128>()
            .0
            .iter()
            .all(|b| b.iter().fold(0u8, |a, &x| a.wrapping_add(x)) == 0)
        && e.as_chunks::<128>()
            .0
            .iter()
            .filter(|b| b[0] == 0x70)
            .all(|b| {
                let end = 5 + b[2] as usize;
                end < 127 && b[1..=end].iter().fold(0u8, |a, &x| a.wrapping_add(x)) == 0
            })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncated_edids_and_extensions_never_escape_bounds() {
        for len in 8..128 {
            let mut img = vec![0; len];
            img[..8].copy_from_slice(&HEADER);
            assert!(find(&img).is_empty());
        }
        let mut img = vec![0; 128];
        img[..8].copy_from_slice(&HEADER);
        img[126] = 1;
        assert!(find(&img).is_empty());
        let mut img = vec![0; 256];
        img[128] = 0x70;
        img[130] = 255;
        assert!(timings(&img).is_empty());
        img[130] = 23;
        img[133] = 3;
        img[135] = 21; // incomplete Type-I timing
        assert!(timings(&img).is_empty());
    }

    #[test]
    fn cta_zero_dtd_offset_does_not_parse_its_header_as_a_timing() {
        let mut img = vec![0; 256];
        img[128] = 2;
        img[129] = 3;
        assert!(timings(&img).is_empty());
    }

    #[test]
    fn checks_inner_displayid_checksum_as_well_as_outer_block_checksum() {
        let mut img = vec![0; 256];
        img[128] = 0x70;
        img[129] = 0x12;
        fix_displayid_checksum(&mut img, 128);
        fix_block_checksums(&mut img);
        assert!(checksums_ok(&img));
        img[129] ^= 1;
        fix_block_checksums(&mut img);
        assert!(!checksums_ok(&img));
        assert!(!checksums_ok(&[]));
        assert!(!checksums_ok(&[0; 127]));
    }
}
