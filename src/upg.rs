//! Dell/Realtek `.UPG` container (verified against UPGDLL.dll and the decompiled Firmware Updater).
//!
//! ```text
//! 0x00  global header (30 bytes)                  u16 sum16 at 0x1E
//! 0x20  block descriptor #0 (0x50 bytes)           image sum16 at +0x3E, descriptor sum16 at +0x4E
//! 0x70  block descriptor #1
//! ...   images; each followed by an info tag: model(9) version(6) panel(1) pd(1) bin-type(1)
//! ```

use std::fmt;
use std::io::{self, ErrorKind};

const HDR_CK: usize = 0x1E;
const DESCS: [usize; 2] = [0x20, 0x70];
pub const BANK: usize = 0x10000;
const SIGNED_BLOCK: usize = 4 + BANK + 64;
const SIG_PUBKEY: usize = 64;
const DESC_END: usize = 0xC0;
const INFO_TAG_LEN: usize = 18;
const ADDRESS_SPACE: usize = 1 << 24;

pub fn sum16(b: &[u8]) -> u16 {
    b.iter().fold(0u16, |s, &x| s.wrapping_add(x as u16))
}

fn u32_at(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}

fn u16_at(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(d[o..o + 2].try_into().unwrap())
}

#[derive(Debug, Clone)]
pub struct Block {
    pub desc: usize,
    pub chip: String,
    pub offset: usize,
    pub len: usize,
    pub flash_start: u32,
    pub complete_addr: u32,
    pub model: String,
    pub panel: char,
    pub version: String,
}

pub struct Upg {
    data: Vec<u8>,
    blocks: Vec<Block>,
}

impl Upg {
    pub fn load(path: &str) -> std::io::Result<Self> {
        Self::from_bytes(std::fs::read(path)?)
            .map_err(|e| io::Error::new(ErrorKind::InvalidData, format!("{path}: {e}")))
    }

    /// Parse the supported two-panel container and validate all ranges before exposing slices.
    /// Checksums remain separate so `upg` can diagnose checksum failures.
    pub fn from_bytes(data: Vec<u8>) -> Result<Self, String> {
        let d = &data;
        if d.len() < DESC_END {
            return Err("truncated UPG header/descriptors (need at least 0xC0 bytes)".into());
        }
        let blocks: Vec<Block> = DESCS
            .iter()
            .map(|&desc| {
                let offset = u32_at(d, desc + 0x1A) as usize;
                let len = u32_at(d, desc + 0x1E) as usize;
                let end = offset
                    .checked_add(len)
                    .and_then(|n| n.checked_add(INFO_TAG_LEN))
                    .ok_or_else(|| format!("descriptor {desc:#x}: image range overflow"))?;
                if offset < DESC_END || end > d.len() {
                    return Err(format!("descriptor {desc:#x}: image/tag outside container"));
                }
                let chip = String::from_utf8_lossy(&d[desc + 1..desc + 0x19])
                    .trim_end_matches('\u{FFFD}')
                    .to_string();
                let tag = &d[offset + len..offset + len + 18];
                Ok(Block {
                    desc,
                    chip,
                    offset,
                    len,
                    flash_start: u32_at(d, desc + 0x36),
                    complete_addr: u32_at(d, desc + 0x3A),
                    model: String::from_utf8_lossy(&tag[..9]).to_string(),
                    version: String::from_utf8_lossy(&tag[9..15]).to_string(),
                    panel: tag[15] as char,
                })
            })
            .collect::<Result<_, String>>()?;
        if blocks[0].panel == blocks[1].panel {
            return Err("duplicate panel IDs".into());
        }
        if blocks[0].offset < blocks[1].offset + blocks[1].len + INFO_TAG_LEN
            && blocks[1].offset < blocks[0].offset + blocks[0].len + INFO_TAG_LEN
        {
            return Err("overlapping image/tag ranges".into());
        }
        let upg = Self { data, blocks };
        for b in &upg.blocks {
            upg.validate_signed_image(b)?;
        }
        Ok(upg)
    }

    fn validate_signed_image(&self, b: &Block) -> Result<(), String> {
        let img = self.image(b);
        if img.len() < SIG_PUBKEY + SIGNED_BLOCK
            || !(img.len() - SIG_PUBKEY).is_multiple_of(SIGNED_BLOCK)
        {
            return Err(format!(
                "panel '{}': invalid signed-bank image length",
                b.panel
            ));
        }
        let count = (img.len() - SIG_PUBKEY) / SIGNED_BLOCK;
        if count > ADDRESS_SPACE / BANK || b.flash_start as usize >= ADDRESS_SPACE {
            return Err("image exceeds 24-bit flash address space".into());
        }
        let mut seen = vec![false; count];
        for record in img[..img.len() - SIG_PUBKEY].as_chunks::<SIGNED_BLOCK>().0 {
            let offset = u32_at(record, 0) as usize;
            let index = offset / BANK;
            if !offset.is_multiple_of(BANK) || index >= count || seen[index] {
                return Err(format!(
                    "panel '{}': duplicate, unaligned or non-contiguous bank offset {offset:#x}",
                    b.panel
                ));
            }
            seen[index] = true;
        }
        if b.flash_start as usize + count * BANK > ADDRESS_SPACE {
            return Err("image exceeds 24-bit flash address space".into());
        }
        Ok(())
    }

    pub fn blocks(&self) -> Vec<Block> {
        self.blocks.clone()
    }

    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    pub fn require_valid_checksums(&self) -> Result<(), String> {
        let errors = self.verify();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(format!("UPG checksum failure: {}", errors.join(", ")))
        }
    }

    pub fn image(&self, b: &Block) -> &[u8] {
        &self.data[b.offset..b.offset + b.len]
    }

    pub fn image_mut(&mut self, b: &Block) -> &mut [u8] {
        &mut self.data[b.offset..b.offset + b.len]
    }

    /// Signed RTK image: N x [u32 LE offset from flash_start][64 KB data][64-byte signature] + 64-byte public key.
    /// Returns (offset, data) per block, in file order.
    pub fn signed_blocks<'a>(&'a self, b: &Block) -> Vec<(u32, &'a [u8])> {
        let img = self.image(b);
        let n = (img.len() - SIG_PUBKEY) / SIGNED_BLOCK;
        (0..n)
            .map(|k| {
                let p = k * SIGNED_BLOCK;
                (u32_at(img, p), &img[p + 4..p + 4 + BANK])
            })
            .collect()
    }

    /// Flash contents of the image's region, offset 0 = `flash_start`, banks in flash order.
    pub fn flash_view(&self, b: &Block) -> Vec<u8> {
        let blocks = self.signed_blocks(b);
        let end = blocks
            .iter()
            .map(|(o, _)| *o as usize + BANK)
            .max()
            .unwrap_or(0);
        let mut v = vec![0xFF; end];
        for (o, d) in blocks {
            v[o as usize..o as usize + BANK].copy_from_slice(d);
        }
        v
    }

    /// Write flash-order bytes back into the container, preserving record order and signature bytes.
    /// Preserving signature bytes does not make modified firmware cryptographically authentic.
    pub fn replace_flash_view(&mut self, b: &Block, view: &[u8]) -> Result<(), String> {
        let img = self.image_mut(b);
        let end = img.len() - SIG_PUBKEY;
        if view.len() != end / SIGNED_BLOCK * BANK {
            return Err("replacement flash-view length differs from image".into());
        }
        for record in img[..end].as_chunks_mut::<SIGNED_BLOCK>().0 {
            let offset = u32_at(record, 0) as usize;
            record[4..4 + BANK].copy_from_slice(&view[offset..offset + BANK]);
        }
        Ok(())
    }

    pub fn verify(&self) -> Vec<String> {
        let d = &self.data;
        let mut errs = vec![];
        if sum16(&d[..HDR_CK]) != u16_at(d, HDR_CK) {
            errs.push("header checksum".into());
        }
        for b in self.blocks() {
            if sum16(self.image(&b)) != u16_at(d, b.desc + 0x3E) {
                errs.push(format!("image sum (desc {:#x})", b.desc));
            }
            if sum16(&d[b.desc..b.desc + 0x4E]) != u16_at(d, b.desc + 0x4E) {
                errs.push(format!("descriptor checksum (desc {:#x})", b.desc));
            }
        }
        errs
    }

    pub fn fix_checksums(&mut self) {
        for b in self.blocks() {
            let s = sum16(self.image(&b));
            self.data[b.desc + 0x3E..b.desc + 0x40].copy_from_slice(&s.to_le_bytes());
            let s = sum16(&self.data[b.desc..b.desc + 0x4E]);
            self.data[b.desc + 0x4E..b.desc + 0x50].copy_from_slice(&s.to_le_bytes());
        }
        let s = sum16(&self.data[..HDR_CK]);
        self.data[HDR_CK..HDR_CK + 2].copy_from_slice(&s.to_le_bytes());
    }

    pub fn save(&mut self, path: &str) -> std::io::Result<()> {
        self.fix_checksums();
        self.require_valid_checksums()
            .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
        std::fs::write(path, &self.data)
    }
}

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {} panel '{}': file {:#08x}+{:#x} -> flash {:#08x}, complete code @ {:#08x}",
            self.chip,
            self.version,
            self.panel,
            self.offset,
            self.len,
            self.flash_start,
            self.complete_addr
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Upg {
        let len = 2 * SIGNED_BLOCK + SIG_PUBKEY;
        let mut data = vec![0; DESC_END + 2 * (len + INFO_TAG_LEN)];
        for (panel, desc) in DESCS.into_iter().enumerate() {
            let offset = DESC_END + panel * (len + INFO_TAG_LEN);
            data[desc + 0x1A..desc + 0x1E].copy_from_slice(&(offset as u32).to_le_bytes());
            data[desc + 0x1E..desc + 0x22].copy_from_slice(&(len as u32).to_le_bytes());
            data[offset + len + 15] = b'0' + panel as u8;
            // Deliberately put bank 1 before bank 0.
            data[offset..offset + 4].copy_from_slice(&(BANK as u32).to_le_bytes());
            data[offset + 4..offset + 4 + BANK].fill(0x11);
            data[offset + SIGNED_BLOCK + 4..offset + SIGNED_BLOCK + 4 + BANK].fill(0x22);
        }
        let mut upg = Upg::from_bytes(data).unwrap();
        upg.fix_checksums();
        upg
    }

    #[test]
    fn rejects_truncated_headers_and_bad_image_ranges() {
        for len in 0..DESC_END {
            assert!(Upg::from_bytes(vec![0; len]).is_err());
        }
        let mut data = fixture().data;
        data[DESCS[0] + 0x1A..DESCS[0] + 0x1E].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Upg::from_bytes(data).is_err());
    }

    #[test]
    fn rejects_duplicate_unaligned_sparse_and_truncated_signed_banks() {
        let original = fixture();
        let b = original.blocks()[0].clone();
        for offset in [0, 1, (3 * BANK) as u32] {
            let mut data = original.data.clone();
            data[b.offset..b.offset + 4].copy_from_slice(&offset.to_le_bytes());
            assert!(Upg::from_bytes(data).is_err());
        }
        let mut data = original.data.clone();
        data[b.desc + 0x1E..b.desc + 0x22].copy_from_slice(&((b.len - 1) as u32).to_le_bytes());
        assert!(Upg::from_bytes(data).is_err());
    }

    #[test]
    fn flash_view_roundtrip_preserves_record_order_and_signatures() {
        let mut upg = fixture();
        let b = upg.blocks()[0].clone();
        let before = upg.image(&b).to_vec();
        let mut view = upg.flash_view(&b);
        assert_eq!(view[0], 0x22);
        assert_eq!(view[BANK], 0x11);
        view[0] = 0x33;
        upg.replace_flash_view(&b, &view).unwrap();
        let differences: Vec<_> = before
            .iter()
            .zip(upg.image(&b))
            .enumerate()
            .filter_map(|(i, (a, b))| (a != b).then_some(i))
            .collect();
        assert_eq!(differences, [SIGNED_BLOCK + 4]);
        upg.fix_checksums();
        assert!(upg.verify().is_empty());
        upg.image_mut(&b)[4] ^= 1;
        assert!(upg.require_valid_checksums().is_err());
    }
}
