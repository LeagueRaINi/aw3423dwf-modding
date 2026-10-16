//! Analysis commands.

use super::{parse_num, parse_u16};
use crate::cli::XrefKind;
use crate::dis51;

/// `dis BANK.bin [ENTRY ...] [--case ADDR] | --range START LEN`: one 64K bank, code address 0 = file offset 0.
/// Cross-reference a whole flash-view image (all 64 KB banks) for either XDATA accesses or code targets.
///   awtool xref FLASHVIEW.bin xdata 0xADDR   every MOV DPTR,#ADDR and whether the nearby MOVX is read/write
///   awtool xref FLASHVIEW.bin code  0xADDR   every LCALL/LJMP/ACALL/AJMP that targets ADDR
/// XDATA is shared across banks, so an access in any bank counts. Offsets are reported bank-local (= fv & 0xFFFF).
pub fn cmd_xref(path: &str, kind: XrefKind, addr: &str) -> Result<(), String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let addr = parse_u16(addr)?;
    let nbanks = data.len().div_ceil(0x10000);
    let mut hits = 0;
    for b in 0..nbanks {
        let base = b * 0x10000;
        let bank = &data[base..(base + 0x10000).min(data.len())];
        let mut i = 0;
        while i + 2 <= bank.len() {
            match kind {
                // MOV DPTR,#addr  (90 hi lo); classify the first MOVX within the next 8 bytes.
                XrefKind::Xdata
                    if i + 3 <= bank.len()
                        && bank[i] == 0x90
                        && u16::from_be_bytes([bank[i + 1], bank[i + 2]]) == addr =>
                {
                    if let Some((rw, wide)) = xdata_access(bank, i + 3) {
                        println!(
                            "  bank {b:#04x}:{:04x}  {rw}{}  [{}]",
                            i,
                            if wide { " (16-bit)" } else { "" },
                            bank[i..i + 11.min(bank.len() - i)]
                                .iter()
                                .map(|x| format!("{x:02x}"))
                                .collect::<Vec<_>>()
                                .join(" ")
                        );
                        hits += 1;
                    }
                }
                // LCALL (12 hi lo) / LJMP (02 hi lo) absolute to addr.
                XrefKind::Code
                    if i + 3 <= bank.len()
                        && (bank[i] == 0x12 || bank[i] == 0x02)
                        && u16::from_be_bytes([bank[i + 1], bank[i + 2]]) == addr =>
                {
                    let op = if bank[i] == 0x12 { "LCALL" } else { "LJMP " };
                    println!("  bank {b:#04x}:{:04x}  {op} {addr:#06x}", i);
                    hits += 1;
                }
                // ACALL/AJMP: 2-byte, opcode low nibble 1, high 3 bits = page; target = (pc+2 & 0xF800)|(page<<8)|arg.
                XrefKind::Code if bank[i] & 0x1F == 0x01 || bank[i] & 0x1F == 0x11 => {
                    let page = (bank[i] >> 5) as u16;
                    let tgt =
                        ((i as u16).wrapping_add(2) & 0xF800) | (page << 8) | bank[i + 1] as u16;
                    if tgt == addr {
                        let op = if bank[i] & 0x1F == 0x11 {
                            "ACALL"
                        } else {
                            "AJMP "
                        };
                        println!("  bank {b:#04x}:{:04x}  {op} {addr:#06x}", i);
                        hits += 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
    println!(
        "{hits} candidate site(s) referencing {}{addr:#06x} (byte scan; confirm code boundaries and active bank)",
        if matches!(kind, XrefKind::Xdata) {
            "XDATA "
        } else {
            "CODE "
        }
    );
    Ok(())
}

pub fn cmd_dis(
    path: &str,
    entries: &[String],
    case: &[String],
    range: Option<&[String]>,
) -> Result<(), String> {
    let mut bank = std::fs::read(path).map_err(|e| e.to_string())?;
    if bank.len() > 0x10000 {
        return Err(format!(
            "{path} is {:#x} bytes; slice one 64K bank first",
            bank.len()
        ));
    }
    bank.resize(0x10000, 0xFF);
    if let Some(r) = range {
        let (start, len) = (parse_u16(&r[0])?, parse_num(&r[1])? as usize);
        if len > 0x10000 - start as usize {
            return Err("disassembly range exceeds the bank".into());
        }
        print!("{}", dis51::render_range(&bank, start, len));
        return Ok(());
    }
    let helpers: Vec<u16> = case
        .iter()
        .map(|a| parse_u16(a))
        .collect::<Result<_, _>>()?;
    let mut entries: Vec<u16> = entries
        .iter()
        .map(|a| parse_u16(a))
        .collect::<Result<_, _>>()?;
    if entries.is_empty() {
        entries = dis51::sweep_entries(&bank);
        eprintln!(
            "no entries given: starting from 0 and {} LCALL/LJMP targets of a linear sweep",
            entries.len() - 1
        );
    }
    print!(
        "{}",
        dis51::analyze(&bank, &entries, &helpers).render(&bank)
    );
    Ok(())
}

/// Decode instructions after a candidate MOV DPTR; operands are not MOVX opcodes.
fn xdata_access(bank: &[u8], start: usize) -> Option<(&'static str, bool)> {
    let end = (start + 8).min(bank.len());
    let (mut at, mut read, mut write, mut offset, mut wide, mut accessed_base) =
        (start, false, false, 0, false, false);
    while at < end {
        let ins = dis51::decode(bank, at as u16);
        if at + ins.len > end || ins.flow != dis51::Flow::Next {
            break;
        }
        let op = bank[at];
        if op == 0x90
            || ins.text.starts_with("MOV DPL,")
            || ins.text.starts_with("MOV DPH,")
            || ins.text.starts_with("POP DPL")
            || ins.text.starts_with("POP DPH")
        {
            break;
        }
        match op {
            0xE0 | 0xF0 if offset <= 1 => {
                accessed_base |= offset == 0;
                if offset == 1 && (read || write) {
                    wide = true;
                }
                read |= op == 0xE0;
                write |= op == 0xF0;
            }
            0xA3 => offset += 1,
            _ => {}
        }
        at += ins.len;
    }
    if !accessed_base {
        return None;
    }
    match (read, write) {
        (true, true) => Some(("read/write", wide)),
        (true, false) => Some(("read", wide)),
        (false, true) => Some(("write", wide)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdata_classifier_skips_operands_and_stops_at_calls_or_dptr_changes() {
        assert_eq!(xdata_access(&[0x74, 0xE0, 0x22], 0), None);
        assert_eq!(xdata_access(&[0x74, 0xF0, 0xE0], 0), Some(("read", false)));
        assert_eq!(xdata_access(&[0xE0, 0xA3, 0xE0], 0), Some(("read", true)));
        assert_eq!(xdata_access(&[0x12, 0x00, 0xE0, 0xF0], 0), None);
        assert_eq!(xdata_access(&[0x90, 0x00, 0x00, 0xE0], 0), None);
        assert_eq!(xdata_access(&[0xA3, 0xE0], 0), None);
    }
}
