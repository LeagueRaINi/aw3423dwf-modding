//! Supported flash layout and interrupted-update marker states.
//! Keep destructive address checks independent of USB transport.

use crate::upg::{BANK, Block};

pub const USER_START: u32 = 0x400000;
pub const USER_LEN: usize = 28 * BANK;
pub const MARKER_ADDR: u32 = 0x7FF8FE;
pub const MARKER_BLOCK: u32 = MARKER_ADDR & !0xFFFF;
pub const MARKER_OFFSET: usize = (MARKER_ADDR & 0xFFFF) as usize;

pub fn validate_target(block: &Block, len: usize) -> Result<(), String> {
    if block.model != "AW3423DWF"
        || block.chip != "RTD2717Q-VDL-CG"
        || block.version != "M3B107"
        || block.flash_start != USER_START
        || len != USER_LEN
        || block.complete_addr != MARKER_ADDR
        || !matches!(block.panel, '0' | '1')
    {
        return Err(format!(
            "unsupported direct-flash layout: {block}, flash-view length {len:#x}"
        ));
    }
    Ok(())
}

/// Permit only this profile's user image and reserved marker block; protect the golden region.
pub fn writable_range(start: u32, len: usize) -> Result<(), String> {
    let end = u32::try_from(len)
        .ok()
        .and_then(|n| start.checked_add(n))
        .ok_or("write range overflow")?;
    if len == 0
        || !((start >= USER_START && end <= USER_START + USER_LEN as u32)
            || (start >= MARKER_BLOCK && end <= MARKER_BLOCK + BANK as u32))
    {
        return Err(format!(
            "write outside supported user/marker regions: {start:#x}+{len:#x}"
        ));
    }
    Ok(())
}

pub fn self_test_pattern(seed: u8) -> Vec<u8> {
    (0..64).map(|i| seed ^ i).collect()
}

pub fn complete_marker_block() -> Vec<u8> {
    let mut bytes = vec![0xFF; BANK];
    bytes[MARKER_OFFSET..MARKER_OFFSET + 2].copy_from_slice(&[0xAA, 0x55]);
    bytes
}

/// Validate every marker-block byte before erasing it. CRC-8 alone cannot prove it is disposable.
/// Allow erased, invalidated, and interrupted self-test states created by this tool.
pub fn validate_marker_block(bytes: &[u8]) -> Result<bool, String> {
    if bytes.len() != BANK {
        return Err("marker block must be exactly 64 KB".into());
    }
    let complete = complete_marker_block();
    if bytes == complete {
        return Ok(true);
    }
    for (at, &byte) in bytes.iter().enumerate() {
        let allowed = match at {
            MARKER_OFFSET => matches!(byte, 0xFF | 0xAA | 0x00),
            n if n == MARKER_OFFSET + 1 => matches!(byte, 0xFF | 0x55 | 0x00),
            0x40..=0x7F => byte == 0xFF || byte == 0x5A ^ (at - 0x40) as u8,
            0x80..=0xBF => byte == 0xFF || byte == 0xA5 ^ (at - 0x80) as u8,
            _ => byte == 0xFF,
        };
        if !allowed {
            return Err(format!(
                "unexpected marker-block data at +{at:#x} ({byte:02x}); refusing erase"
            ));
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_marker_recovery_states_and_rejects_unrelated_data() {
        assert_eq!(validate_marker_block(&complete_marker_block()), Ok(true));
        let mut b = vec![0xFF; BANK];
        assert_eq!(validate_marker_block(&b), Ok(false));
        b[MARKER_OFFSET] = 0;
        b[MARKER_OFFSET + 1] = 0x55; // interruption between invalidation bytes
        b[0x40..0x80].copy_from_slice(&self_test_pattern(0x5A));
        b[0x80..0xA0].copy_from_slice(&self_test_pattern(0xA5)[..32]);
        assert_eq!(validate_marker_block(&b), Ok(false));
        b[0x100] = 0;
        assert!(validate_marker_block(&b).is_err());
    }

    #[test]
    fn writes_cannot_reach_golden_firmware_or_cross_region_boundaries() {
        for (start, len) in [
            (0, BANK),
            (USER_START - 1, 2),
            (USER_START, USER_LEN + 1),
            (MARKER_BLOCK, BANK + 1),
            (u32::MAX, 2),
            (USER_START, 0),
        ] {
            assert!(writable_range(start, len).is_err());
        }
        assert!(writable_range(USER_START, USER_LEN).is_ok());
        assert!(writable_range(MARKER_BLOCK, BANK).is_ok());
    }
}
