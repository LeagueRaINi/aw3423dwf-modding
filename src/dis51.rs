//! MCS-51 disassembler for one 64 KB RTD2717Q code bank: full opcode table, recursive descent over
//! calls/jumps/branches, Keil C51 `?C?CCASE` switch tables, function grouping and CODE XREF comments.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Next,
    Jump(u16),
    Call(u16),
    Branch(u16),
    Return,
    Indirect,
}

pub struct Insn {
    pub len: usize,
    pub text: String,
    pub flow: Flow,
}

const SFRS: [(u8, &str); 21] = [
    (0x80, "P0"),
    (0x81, "SP"),
    (0x82, "DPL"),
    (0x83, "DPH"),
    (0x87, "PCON"),
    (0x88, "TCON"),
    (0x89, "TMOD"),
    (0x8A, "TL0"),
    (0x8B, "TL1"),
    (0x8C, "TH0"),
    (0x8D, "TH1"),
    (0x90, "P1"),
    (0x98, "SCON"),
    (0x99, "SBUF"),
    (0xA0, "P2"),
    (0xA8, "IE"),
    (0xB0, "P3"),
    (0xB8, "IP"),
    (0xD0, "PSW"),
    (0xE0, "ACC"),
    (0xF0, "B"),
];

fn direct(d: u8) -> String {
    match SFRS.iter().find(|(a, _)| *a == d) {
        Some((_, n)) => n.to_string(),
        None => format!("0x{d:02x}"),
    }
}

fn bit(b: u8) -> String {
    if b < 0x80 {
        format!("0x{:02x}.{}", 0x20 + (b >> 3), b & 7)
    } else {
        format!("{}.{}", direct(b & 0xF8), b & 7)
    }
}

/// Decode the instruction at `pc` (the bank wraps at 64 KB, so a truncated tail never panics).
pub fn decode(bank: &[u8], pc: u16) -> Insn {
    let at = |i: usize| bank[(pc as usize + i) % bank.len()];
    let (op, b1, b2) = (at(0), at(1), at(2));
    let lo = op & 0x0F;
    let a16 = u16::from_be_bytes([b1, b2]);
    let a11 = (pc.wrapping_add(2) & 0xF800) | (((op >> 5) as u16) << 8) | b1 as u16;
    let rel = |len: u16, off: u8| pc.wrapping_add(len).wrapping_add(off as i8 as u16);
    let imm = |v: u8| format!("#0x{v:02x}");
    let ins = |len: usize, text: String| Insn {
        len,
        text,
        flow: Flow::Next,
    };
    let fl = |len: usize, text: String, flow: Flow| Insn { len, text, flow };
    let br2 = |m: &str| {
        fl(
            2,
            format!("{m} 0x{:04x}", rel(2, b1)),
            Flow::Branch(rel(2, b1)),
        )
    };
    let bitbr = |m: &str| {
        fl(
            3,
            format!("{m} {},0x{:04x}", bit(b1), rel(3, b2)),
            Flow::Branch(rel(3, b2)),
        )
    };

    if lo < 4 {
        return match op {
            0x00 => ins(1, "NOP".into()),
            0x10 => bitbr("JBC"),
            0x20 => bitbr("JB"),
            0x30 => bitbr("JNB"),
            0x40 => br2("JC"),
            0x50 => br2("JNC"),
            0x60 => br2("JZ"),
            0x70 => br2("JNZ"),
            0x80 => fl(
                2,
                format!("SJMP 0x{:04x}", rel(2, b1)),
                Flow::Jump(rel(2, b1)),
            ),
            0x90 => ins(3, format!("MOV DPTR,#0x{a16:04x}")),
            0xA0 => ins(2, format!("ORL C,/{}", bit(b1))),
            0xB0 => ins(2, format!("ANL C,/{}", bit(b1))),
            0xC0 => ins(2, format!("PUSH {}", direct(b1))),
            0xD0 => ins(2, format!("POP {}", direct(b1))),
            0xE0 => ins(1, "MOVX A,@DPTR".into()),
            0xF0 => ins(1, "MOVX @DPTR,A".into()),
            _ if lo == 1 && op & 0x10 == 0 => fl(2, format!("AJMP 0x{a11:04x}"), Flow::Jump(a11)),
            _ if lo == 1 => fl(2, format!("ACALL 0x{a11:04x}"), Flow::Call(a11)),
            0x02 => fl(3, format!("LJMP 0x{a16:04x}"), Flow::Jump(a16)),
            0x12 => fl(3, format!("LCALL 0x{a16:04x}"), Flow::Call(a16)),
            0x22 => fl(1, "RET".into(), Flow::Return),
            0x32 => fl(1, "RETI".into(), Flow::Return),
            0x42 => ins(2, format!("ORL {},A", direct(b1))),
            0x52 => ins(2, format!("ANL {},A", direct(b1))),
            0x62 => ins(2, format!("XRL {},A", direct(b1))),
            0x72 => ins(2, format!("ORL C,{}", bit(b1))),
            0x82 => ins(2, format!("ANL C,{}", bit(b1))),
            0x92 => ins(2, format!("MOV {},C", bit(b1))),
            0xA2 => ins(2, format!("MOV C,{}", bit(b1))),
            0xB2 => ins(2, format!("CPL {}", bit(b1))),
            0xC2 => ins(2, format!("CLR {}", bit(b1))),
            0xD2 => ins(2, format!("SETB {}", bit(b1))),
            0xE2 => ins(1, "MOVX A,@R0".into()),
            0xF2 => ins(1, "MOVX @R0,A".into()),
            0x03 => ins(1, "RR A".into()),
            0x13 => ins(1, "RRC A".into()),
            0x23 => ins(1, "RL A".into()),
            0x33 => ins(1, "RLC A".into()),
            0x43 => ins(3, format!("ORL {},{}", direct(b1), imm(b2))),
            0x53 => ins(3, format!("ANL {},{}", direct(b1), imm(b2))),
            0x63 => ins(3, format!("XRL {},{}", direct(b1), imm(b2))),
            0x73 => fl(1, "JMP @A+DPTR".into(), Flow::Indirect),
            0x83 => ins(1, "MOVC A,@A+PC".into()),
            0x93 => ins(1, "MOVC A,@A+DPTR".into()),
            0xA3 => ins(1, "INC DPTR".into()),
            0xB3 => ins(1, "CPL C".into()),
            0xC3 => ins(1, "CLR C".into()),
            0xD3 => ins(1, "SETB C".into()),
            0xE3 => ins(1, "MOVX A,@R1".into()),
            _ => ins(1, "MOVX @R1,A".into()),
        };
    }

    // Columns 4..F: A / direct / @Ri / Rn forms of one operation.
    let reg = if lo < 8 {
        format!("@R{}", op & 1)
    } else {
        format!("R{}", op & 7)
    };
    let alu = |m: &str| match lo {
        4 => ins(2, format!("{m} A,{}", imm(b1))),
        5 => ins(2, format!("{m} A,{}", direct(b1))),
        _ => ins(1, format!("{m} A,{reg}")),
    };
    match op >> 4 {
        0x0 => match lo {
            4 => ins(1, "INC A".into()),
            5 => ins(2, format!("INC {}", direct(b1))),
            _ => ins(1, format!("INC {reg}")),
        },
        0x1 => match lo {
            4 => ins(1, "DEC A".into()),
            5 => ins(2, format!("DEC {}", direct(b1))),
            _ => ins(1, format!("DEC {reg}")),
        },
        0x2 => alu("ADD"),
        0x3 => alu("ADDC"),
        0x4 => alu("ORL"),
        0x5 => alu("ANL"),
        0x6 => alu("XRL"),
        0x9 => alu("SUBB"),
        0x7 => match lo {
            4 => ins(2, format!("MOV A,{}", imm(b1))),
            5 => ins(3, format!("MOV {},{}", direct(b1), imm(b2))),
            _ => ins(2, format!("MOV {reg},{}", imm(b1))),
        },
        0x8 => match lo {
            4 => ins(1, "DIV AB".into()),
            5 => ins(3, format!("MOV {},{}", direct(b2), direct(b1))),
            _ => ins(2, format!("MOV {},{reg}", direct(b1))),
        },
        0xA => match lo {
            4 => ins(1, "MUL AB".into()),
            5 => ins(1, "DB 0xa5".into()),
            _ => ins(2, format!("MOV {reg},{}", direct(b1))),
        },
        0xB => {
            let t = rel(3, b2);
            match lo {
                4 => fl(3, format!("CJNE A,{},0x{t:04x}", imm(b1)), Flow::Branch(t)),
                5 => fl(
                    3,
                    format!("CJNE A,{},0x{t:04x}", direct(b1)),
                    Flow::Branch(t),
                ),
                _ => fl(
                    3,
                    format!("CJNE {reg},{},0x{t:04x}", imm(b1)),
                    Flow::Branch(t),
                ),
            }
        }
        0xC => match lo {
            4 => ins(1, "SWAP A".into()),
            5 => ins(2, format!("XCH A,{}", direct(b1))),
            _ => ins(1, format!("XCH A,{reg}")),
        },
        0xD => match lo {
            4 => ins(1, "DA A".into()),
            5 => fl(
                3,
                format!("DJNZ {},0x{:04x}", direct(b1), rel(3, b2)),
                Flow::Branch(rel(3, b2)),
            ),
            6 | 7 => ins(1, format!("XCHD A,{reg}")),
            _ => fl(
                2,
                format!("DJNZ {reg},0x{:04x}", rel(2, b1)),
                Flow::Branch(rel(2, b1)),
            ),
        },
        0xE => match lo {
            4 => ins(1, "CLR A".into()),
            5 => ins(2, format!("MOV A,{}", direct(b1))),
            _ => ins(1, format!("MOV A,{reg}")),
        },
        _ => match lo {
            4 => ins(1, "CPL A".into()),
            5 => ins(2, format!("MOV {},A", direct(b1))),
            _ => ins(1, format!("MOV {reg},A")),
        },
    }
}

/// A Keil C51 switch table that follows `LCALL ?C?CCASE`.
pub struct CaseTable {
    pub at: u16,
    pub end: u16,
    pub cases: Vec<(u8, u16)>,
    pub default: u16,
    pub layout: &'static str,
}

fn parse_layout(bank: &[u8], at: u16, val_first: bool) -> Option<CaseTable> {
    let mut p = at as usize;
    let mut cases = Vec::new();
    while p + 4 <= bank.len() && cases.len() < 256 {
        if bank[p] == 0 && bank[p + 1] == 0 {
            let default = u16::from_be_bytes([bank[p + 2], bank[p + 3]]);
            if cases.is_empty() || default == 0 {
                return None;
            }
            let layout = if val_first {
                "[val hi lo]"
            } else {
                "[hi lo val]"
            };
            return Some(CaseTable {
                at,
                end: (p + 4) as u16,
                cases,
                default,
                layout,
            });
        }
        let (v, a) = if val_first {
            (bank[p], u16::from_be_bytes([bank[p + 1], bank[p + 2]]))
        } else {
            (bank[p + 2], u16::from_be_bytes([bank[p], bank[p + 1]]))
        };
        if a == 0 {
            return None;
        }
        cases.push((v, a));
        p += 3;
    }
    None
}

/// Try both entry layouts; the real one has its targets clustered (cases of one function).
pub fn parse_case_table(bank: &[u8], at: u16) -> Option<CaseTable> {
    let spread = |t: &CaseTable| {
        let (mut lo, mut hi) = (t.default, t.default);
        for &(_, a) in &t.cases {
            lo = lo.min(a);
            hi = hi.max(a);
        }
        hi - lo
    };
    match (parse_layout(bank, at, false), parse_layout(bank, at, true)) {
        (Some(a), Some(b)) => Some(if spread(&b) < spread(&a) { b } else { a }),
        (a, b) => a.or(b),
    }
}

/// `?C?CCASE` and friends start by popping their return address: POP DPH / POP DPL.
fn is_case_helper(bank: &[u8], t: u16) -> bool {
    bank.get(t as usize..t as usize + 4) == Some(&[0xD0, 0x83, 0xD0, 0x82])
}

pub struct Func {
    pub start: u16,
    pub end: u16,
    pub insns: usize,
    pub calls: BTreeSet<u16>,
    pub dptr: BTreeSet<u16>,
}

pub struct Disasm {
    pub insns: BTreeMap<u16, Insn>,
    pub tables: BTreeMap<u16, CaseTable>,
    pub xrefs: BTreeMap<u16, Vec<(u16, &'static str)>>,
    pub funcs: Vec<Func>,
}

/// Recursive descent from `entries`; `helpers` are extra switch-helper addresses (auto-detected otherwise).
pub fn analyze(bank: &[u8], entries: &[u16], helpers: &[u16]) -> Disasm {
    let mut d = Disasm {
        insns: BTreeMap::new(),
        tables: BTreeMap::new(),
        xrefs: BTreeMap::new(),
        funcs: Vec::new(),
    };
    let mut starts: BTreeSet<u16> = BTreeSet::new();
    let mut work: Vec<u16> = entries.to_vec();
    while let Some(mut pc) = work.pop() {
        while !d.insns.contains_key(&pc) {
            let ins = decode(bank, pc);
            let next = pc.wrapping_add(ins.len as u16);
            let flow = ins.flow;
            d.insns.insert(pc, ins);
            let mut xref = |t: u16, kind: &'static str| {
                d.xrefs.entry(t).or_default().push((pc, kind));
                work.push(t);
            };
            match flow {
                Flow::Next => pc = next,
                Flow::Jump(t) => {
                    xref(t, "jump");
                    break;
                }
                Flow::Branch(t) => {
                    xref(t, "branch");
                    pc = next;
                }
                Flow::Call(t) => {
                    xref(t, "call");
                    starts.insert(t);
                    let table = if helpers.contains(&t) || is_case_helper(bank, t) {
                        parse_case_table(bank, next)
                    } else {
                        None
                    };
                    match table {
                        Some(tab) => {
                            for &(_, a) in &tab.cases {
                                xref(a, "case");
                            }
                            xref(tab.default, "default");
                            d.tables.insert(next, tab);
                            break;
                        }
                        None => pc = next,
                    }
                }
                Flow::Return | Flow::Indirect => break,
            }
        }
    }

    // Group into functions: flood from each call target over non-call edges, never entering another
    // call target; entries that no function reached (reset vector, sweep hints) become functions last.
    let mut owned: BTreeSet<u16> = BTreeSet::new();
    let roots: Vec<u16> = starts.iter().chain(entries.iter()).copied().collect();
    for s in roots {
        if owned.contains(&s) {
            continue;
        }
        let mut f = Func {
            start: s,
            end: s,
            insns: 0,
            calls: BTreeSet::new(),
            dptr: BTreeSet::new(),
        };
        let mut stack = vec![s];
        while let Some(pc) = stack.pop() {
            if owned.contains(&pc) || (pc != s && starts.contains(&pc)) {
                continue;
            }
            let Some(ins) = d.insns.get(&pc) else {
                continue;
            };
            owned.insert(pc);
            f.insns += 1;
            let next = pc.wrapping_add(ins.len as u16);
            f.end = f.end.max(next);
            if let Some(h) = ins.text.strip_prefix("MOV DPTR,#0x")
                && let Ok(v) = u16::from_str_radix(h, 16)
            {
                f.dptr.insert(v);
            }
            match ins.flow {
                Flow::Next => stack.push(next),
                Flow::Jump(t) => stack.push(t),
                Flow::Branch(t) => {
                    stack.push(t);
                    stack.push(next);
                }
                Flow::Call(t) => {
                    f.calls.insert(t);
                    match d.tables.get(&next) {
                        Some(tab) => {
                            stack.extend(tab.cases.iter().map(|c| c.1));
                            stack.push(tab.default);
                            f.end = f.end.max(tab.end);
                        }
                        None => stack.push(next),
                    }
                }
                Flow::Return | Flow::Indirect => {}
            }
        }
        if f.insns > 0 {
            d.funcs.push(f);
        }
    }
    d.funcs.sort_by_key(|f| f.start);
    d
}

fn hexbytes(bank: &[u8], pc: u16, len: usize) -> String {
    (0..len)
        .map(|i| format!("{:02x}", bank[(pc as usize + i) % bank.len()]))
        .collect::<Vec<_>>()
        .join(" ")
}

fn flow_note(flow: Flow) -> &'static str {
    match flow {
        Flow::Next => "",
        Flow::Jump(_) => "  ; jump",
        Flow::Call(_) => "  ; call",
        Flow::Branch(_) => "  ; branch",
        Flow::Return => "  ; ret",
        Flow::Indirect => "  ; indirect (stop)",
    }
}

pub fn line(bank: &[u8], pc: u16, ins: &Insn) -> String {
    format!(
        "{pc:04x}: {:<9} {}{}",
        hexbytes(bank, pc, ins.len),
        ins.text,
        flow_note(ins.flow)
    )
}

fn list(set: &BTreeSet<u16>) -> String {
    set.iter()
        .map(|v| format!("{v:04x}"))
        .collect::<Vec<_>>()
        .join(",")
}

impl Disasm {
    pub fn render(&self, bank: &[u8]) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "; {} functions, {} instructions, {} switch tables",
            self.funcs.len(),
            self.insns.len(),
            self.tables.len()
        );
        for f in &self.funcs {
            let _ = write!(
                out,
                "; func 0x{:04x}-0x{:04x} {:4} insns",
                f.start, f.end, f.insns
            );
            if !f.calls.is_empty() {
                let _ = write!(out, "  calls: {}", list(&f.calls));
            }
            if !f.dptr.is_empty() {
                let _ = write!(out, "  dptr: {}", list(&f.dptr));
            }
            out.push('\n');
        }
        let starts: BTreeSet<u16> = self.funcs.iter().map(|f| f.start).collect();
        let mut items: Vec<u16> = self
            .insns
            .keys()
            .chain(self.tables.keys())
            .copied()
            .collect();
        items.sort_unstable();
        items.dedup();
        let mut prev_end: Option<u16> = None;
        for pc in items {
            if let Some(e) = prev_end
                && pc > e
            {
                let _ = writeln!(out, "; ... {} bytes not reached", pc - e);
            }
            if starts.contains(&pc) {
                let _ = writeln!(out, "\n; ===== func 0x{pc:04x} =====");
            }
            if let Some(refs) = self.xrefs.get(&pc) {
                let r: Vec<String> = refs
                    .iter()
                    .map(|(from, k)| format!("0x{from:04x} ({k})"))
                    .collect();
                let _ = writeln!(out, "; CODE XREF from {}", r.join(", "));
            }
            if let Some(tab) = self.tables.get(&pc) {
                let _ = writeln!(
                    out,
                    "; switch table {} with {} cases",
                    tab.layout,
                    tab.cases.len()
                );
                let mut p = tab.at;
                for &(v, a) in &tab.cases {
                    let _ = writeln!(
                        out,
                        "{p:04x}: {:<9} case 0x{v:02x} -> 0x{a:04x}",
                        hexbytes(bank, p, 3)
                    );
                    p = p.wrapping_add(3);
                }
                let _ = writeln!(
                    out,
                    "{p:04x}: {:<9} default -> 0x{:04x}",
                    hexbytes(bank, p, 4),
                    tab.default
                );
                prev_end = Some(tab.end);
                continue;
            }
            let ins = &self.insns[&pc];
            let _ = writeln!(out, "{}", line(bank, pc, ins));
            prev_end = Some(pc.wrapping_add(ins.len as u16));
        }
        out
    }
}

/// Plain linear disassembly of `len` bytes from `start`.
pub fn render_range(bank: &[u8], start: u16, len: usize) -> String {
    let mut out = String::new();
    let (mut pc, end) = (start as usize, (start as usize + len).min(bank.len()));
    while pc < end {
        let ins = decode(bank, pc as u16);
        let _ = writeln!(out, "{}", line(bank, pc as u16, &ins));
        pc += ins.len;
    }
    out
}

/// Entry points for a bank with no hints: the reset vector plus every LCALL/LJMP target of a linear sweep.
pub fn sweep_entries(bank: &[u8]) -> Vec<u16> {
    let mut set = BTreeSet::from([0u16]);
    let mut pc = 0usize;
    while pc < bank.len() {
        let ins = decode(bank, pc as u16);
        if let Flow::Call(t) | Flow::Jump(t) = ins.flow
            && matches!(bank[pc], 0x02 | 0x12)
        {
            set.insert(t);
        }
        pc += ins.len;
    }
    set.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_branch_and_short_call_wrap_at_the_16_bit_boundary() {
        let mut bank = vec![0; 65536];
        bank[..2].copy_from_slice(&[0x80, 0xFE]);
        assert!(matches!(decode(&bank, 0).flow, Flow::Jump(0)));
        bank[65535] = 0x11;
        bank[0] = 0x23;
        assert!(matches!(decode(&bank, 65535).flow, Flow::Call(0x23)));
    }

    #[test]
    fn direct_to_direct_move_decodes_destination_before_source() {
        assert_eq!(decode(&[0x85, 0x82, 0x83], 0).text, "MOV DPH,DPL");
    }
}
