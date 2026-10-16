//! Command handlers and shared argument parsing.

mod analysis;
mod ddc;
mod device;
mod files;
mod flash;
mod isp;
mod patch;

pub use analysis::*;
pub use ddc::*;
pub use files::*;
pub use flash::cmd_flash;
pub use isp::cmd_isp;
pub use patch::*;

fn parse_num(s: &str) -> Result<u32, String> {
    let r = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u32::from_str_radix(h, 16),
        None => s.parse(),
    };
    r.map_err(|_| format!("bad number: {s}"))
}

fn parse_u16(s: &str) -> Result<u16, String> {
    u16::try_from(parse_num(s)?).map_err(|_| format!("address out of 16-bit range: {s}"))
}

fn parse_u8(s: &str) -> Result<u8, String> {
    u8::try_from(parse_num(s)?).map_err(|_| format!("value out of 8-bit range: {s}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_arguments_do_not_silently_truncate() {
        assert_eq!(parse_num("0XFFFF"), Ok(65535));
        assert_eq!(parse_num("165"), Ok(165));
        assert!(parse_u16("0x10000").is_err());
        assert!(parse_u8("0x100").is_err());
    }
}
