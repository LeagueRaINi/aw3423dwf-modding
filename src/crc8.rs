//! Realtek ISP CRC-8 via the `crc` crate. The scaler's hardware CRC engine and
//! `RtkScaler.RTK_CalculateByteCRC` use poly 0x07, init 0, non-reflected, xorout 0 — i.e. CRC-8/SMBUS.

use crc::{CRC_8_SMBUS, Crc};

static CRC8: Crc<u8> = Crc::<u8>::new(&CRC_8_SMBUS);

pub fn crc8(data: &[u8]) -> u8 {
    CRC8.checksum(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_block_matches_dell_erase_check() {
        // A fully-erased 64 KB block reads 0xFF; Dell's blank-check expects CRC 0xDE.
        assert_eq!(crc8(&[0xFF; 65536]), 0xDE);
    }

    #[test]
    fn matching_prefix_crc_is_not_a_monotonic_predicate() {
        // The second byte cancels the first differing byte's CRC syndrome.
        assert_ne!(crc8(&[0]), crc8(&[1]));
        assert_eq!(crc8(&[0, 0]), crc8(&[1, 7]));
    }
}
