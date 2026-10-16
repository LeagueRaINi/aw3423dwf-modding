//! Ddc commands.

use super::parse_u8;
use crate::cli::DdcCmd;
use crate::ddc;

pub fn cmd_ddc(sub: &DdcCmd) -> Result<(), String> {
    let mons = ddc::Monitors::enumerate()?;
    for m in mons.iter() {
        println!("monitor '{}'", m.name());
        match sub {
            DdcCmd::Caps => match m.capabilities() {
                Ok(c) => println!("  {c}"),
                Err(e) => println!("  {e}"),
            },
            DdcCmd::Scan => {
                let mut n = 0;
                for code in 0..=0xFFu8 {
                    if let Some((kind, cur, max)) = m.vcp(code)
                        && (cur != 0 || max != 0)
                    {
                        println!(
                            "  vcp 0x{code:02x}: kind={kind} cur={cur:#x} ({cur}) max={max:#x} ({max})"
                        );
                        n += 1;
                    }
                }
                println!("  {n} codes replied");
            }
            DdcCmd::Get { code } => {
                let code = parse_u8(code)?;
                match m.vcp(code) {
                    Some((kind, cur, max)) => println!(
                        "  vcp 0x{code:02x}: kind={kind} cur={cur:#x} ({cur}) max={max:#x} ({max})"
                    ),
                    None => println!("  vcp 0x{code:02x}: no reply"),
                }
            }
        }
    }
    Ok(())
}
