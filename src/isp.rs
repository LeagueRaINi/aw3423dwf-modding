//! Read-only access to the RTD2717Q's ISP interface (I2C slave 0x4A) and DDC/CI, through the monitor's hub.
//! Register usage mirrors Dell's updater. Erase/program operations require explicit arming;
//! unarmed sessions apply a read-only register/opcode whitelist.

use crate::hid::HidBackend;
use std::thread::sleep;
use std::time::Duration;

const ISP: u8 = 0x4A;
const DDC: u8 = 0x37;
const SPEED: u8 = 2;

/// Hub IDs probed by Dell's updater (the AW3423DWF enumerates as 187C:100E).
const IDS: [(u16, u16); 4] = [
    (0x0BDA, 0x1100),
    (0x0BDA, 0x1101),
    (0x187C, 0x100E),
    (0x187C, 0x1010),
];

/// ISP registers this tool may write. Notably absent: 0x6D/0x70/0x71 (program engine) and anything else.
const WRITABLE: [u8; 18] = [
    0x60, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x6A, 0x6B, 0x6C, 0x6F, 0x72, 0x73, 0x74, 0xED, 0xEE,
    0xF4, 0xF5,
];
/// SPI opcodes this tool may execute: RDID, RES, READ, RDSR, RDSR2.
const SPI_READ_OPS: [u8; 5] = [0x9F, 0xAB, 0x03, 0x05, 0x35];

pub type Result<T> = std::result::Result<T, String>;

pub struct Hub {
    hid: Box<dyn HidBackend>,
    /// The OEM key this hub accepted (see [`crate::keys`]).
    key: Vec<u8>,
    /// When false (default), the register guard forbids every erase/program operation. Flashing sets it true.
    armed: std::cell::Cell<bool>,
    isp_entered: std::cell::Cell<bool>,
}

impl Hub {
    pub fn open(hid: Box<dyn HidBackend>, keys: Vec<Vec<u8>>) -> Result<Hub> {
        for (vid, pid) in IDS {
            if hid.count(vid, pid) <= 0 {
                hid.release_list();
                continue;
            }
            eprintln!("hub {vid:04X}:{pid:04X}");
            if !hid.open(0) {
                hid.release_list();
                continue;
            }
            // Dell's key probe: DDC/CI "get VCP C8"
            let probe = [0x51, 0x82, 0x01, 0xC8, 0x74];
            for (i, k) in keys.into_iter().enumerate() {
                if hid.handshake(&k) == 0 && hid.write(&probe, 0x6E, 0) == 0 {
                    eprintln!("handshake key #{i}");
                    return Ok(Hub {
                        hid,
                        key: k,
                        armed: std::cell::Cell::new(false),
                        isp_entered: std::cell::Cell::new(false),
                    });
                }
            }
            hid.release_list();
            hid.close();
            return Err("no OEM handshake key accepted".into());
        }
        Err("monitor hub not found (is the USB upstream cable connected?)".into())
    }

    /// The hub requires a fresh rolling handshake immediately before every privileged I2C op
    /// (confirmed empirically: skipping or batching it fails with a USB error and halts the MCU).
    fn auth(&self) -> Result<()> {
        match self.hid.handshake(&self.key) {
            0 => Ok(()),
            r => Err(format!("handshake failed ({r})")),
        }
    }

    fn write(&self, slave: u8, sub: u8, data: &[u8]) -> Result<()> {
        let mut buf = vec![sub];
        buf.extend_from_slice(data);
        self.auth()?;
        match self.hid.write(&buf, slave << 1, SPEED) {
            0 => Ok(()),
            r => Err(format!("i2c write {slave:02X}/{sub:02X} failed ({r})")),
        }
    }

    fn read(&self, slave: u8, sub: u8, out: &mut [u8]) -> Result<()> {
        self.auth()?;
        match self.hid.read(&[sub], out, slave << 1, SPEED) {
            0 => Ok(()),
            r => Err(format!("i2c read {slave:02X}/{sub:02X} failed ({r})")),
        }
    }

    // ---------------- DDC/CI ----------------

    fn ddc(&self, cmd: &[u8], reply_len: usize) -> Result<Vec<u8>> {
        let mut rep = vec![0u8; reply_len];
        for _ in 0..5 {
            if self.write(DDC, 0x51, cmd).is_err() {
                continue;
            }
            sleep(Duration::from_millis(500));
            if self.read(DDC, 0x51, &mut rep).is_ok() {
                return Ok(rep);
            }
        }
        Err("no DDC/CI reply".into())
    }

    /// Dell getPanel(): selects which image of a UPG applies to this unit. Validates the DDC/CI reply
    /// header (byte1 == 0x86, like Dell's `reply[1]-128 == 6`) and retries, so a wedged/garbage read
    /// (e.g. after an AMD driver reset) doesn't return a bogus panel id. Reboots the scaler once to
    /// un-wedge DDC if needed.
    pub fn panel_id(&self) -> Result<u8> {
        for attempt in 0..12 {
            if let Ok(rep) = self.ddc(&[0x83, 0x01, 0xFF, 0xFC, 0xBE], 9)
                && rep[1] == 0x86
                && rep[7] != 0
            {
                return Ok(rep[7]);
            }
            if attempt == 5 {
                eprintln!("panel id unreadable (DDC wedged); rebooting scaler to recover ...");
                self.reboot();
                sleep(Duration::from_secs(5));
            }
        }
        Err("could not read a valid panel id over DDC (control channel wedged; try `isp reboot` or power-cycle the monitor)".into())
    }

    /// Dell getUPGVersion() decoding of VCP C8/C9.
    pub fn version(&self) -> Result<String> {
        let c8 = self.ddc(&[0x82, 0x01, 0xC8, 0x74], 11)?;
        let c9 = self.ddc(&[0x82, 0x01, 0xC9, 0x75], 11)?;
        let (hi, lo) = (c9[8], c9[9]);
        let series = match hi & 0x70 {
            0x00 => "0",
            0x10 => "1",
            0x20 => "2",
            0x30 => "3",
            0x40 => "M",
            0x50 => "T",
            0x60 => "S",
            _ => "?",
        };
        let vendor = match c8[9] {
            5 => "2",
            9 => "3",
            _ => "?",
        };
        let v = format!("{series}{vendor}B{:X}{:02X}", hi & 0x0F, lo);
        Ok(if hi & 0x80 != 0 { format!("T{v}") } else { v })
    }

    // ---------------- ISP ----------------

    fn wreg(&self, reg: u8, val: u8) -> Result<()> {
        // When armed for flashing, the erase/program registers and opcodes are permitted; otherwise the
        // guard below makes any write/erase impossible, so read paths can never damage the flash.
        if !self.armed.get() {
            if !WRITABLE.contains(&reg) {
                return Err(format!("refusing to write ISP reg {reg:02X}"));
            }
            if reg == 0x61 && !SPI_READ_OPS.contains(&val) {
                return Err(format!("refusing SPI opcode {val:02X}"));
            }
            // 0x60 = type<<5 | writes<<3 | reads<<1 | exec. Types: 1 write, 2 read, 3 WREN, 4 EWSR, 5 erase.
            if reg == 0x60 && val >> 5 != 2 {
                return Err(format!("refusing non-read common command {val:02X}"));
            }
            if reg == 0x6F && val & 0x60 != 0 {
                return Err("refusing program-engine bits in reg 6F".into());
            }
        }
        let mut last = String::new();
        for _ in 0..5 {
            match self.write(ISP, reg, &[val]) {
                Ok(()) => return Ok(()),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    fn rreg(&self, reg: u8) -> Result<u8> {
        let mut b = [0u8];
        let mut last = String::new();
        for _ in 0..5 {
            match self.read(ISP, reg, &mut b) {
                Ok(()) => return Ok(b[0]),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    fn wait(&self, reg: u8, mask: u8, target: u8, tries: u32) -> Result<()> {
        for _ in 0..tries {
            if self.rreg(reg)? & mask == target & mask {
                return Ok(());
            }
        }
        Err(format!("timeout waiting on reg {reg:02X}"))
    }

    /// RTK_SerialFlashIntoISPMode + RTK_SetDefaultValueToReg. Halts the scaler MCU: the video datapath
    /// keeps running so the picture stays up, but the OSD, buttons and DDC (all MCU-served) freeze until reboot.
    pub fn enter_isp(&self) -> Result<()> {
        let v = self.rreg(0x6F)?;
        self.isp_entered.set(true); // even a failed write may have halted the MCU
        self.wreg(0x6F, (v & 0x7F) | 0x80)?;
        self.wait(0x6F, 0x80, 0x80, 50)?;
        for (r, v) in [
            (0xF4, 0x9F),
            (0xF5, 0x06),
            (0xF4, 0xA0),
            (0xF5, 0x74),
            (0x62, 0x06),
            (0x63, 0x50),
            (0x6A, 0x03),
            (0x6B, 0x0B),
            (0x6C, 0x00),
            (0xED, 0x88),
            (0xEE, 0x04),
        ] {
            self.wreg(r, v)?;
        }
        Ok(())
    }

    /// RTK_Reboot. The hub may drop the link while the scaler restarts, so errors after the write are expected.
    pub fn reboot(&self) {
        self.isp_entered.set(false);
        match self.rreg(0xEE).and_then(|v| self.wreg(0xEE, v | 0x02)) {
            Ok(()) => eprintln!("scaler reboot requested"),
            Err(e) => eprintln!(
                "reboot write reported: {e} (expected: the reboot drops the link mid-write; verify the OSD responds again)"
            ),
        }
    }

    fn spi_common(&self, op: u8, addr: Option<u32>, n_read: u8) -> Result<Vec<u8>> {
        let n_write = if addr.is_some() { 3 } else { 0 };
        let ctl = 0x40 | (n_write << 3) | (n_read << 1); // RDID -> 0x46, RES -> 0x5A (as in Dell's code)
        self.wreg(0x60, ctl)?;
        self.wreg(0x61, op)?;
        if let Some(a) = addr {
            self.wreg(0x64, (a >> 16) as u8)?;
            self.wreg(0x65, (a >> 8) as u8)?;
            self.wreg(0x66, a as u8)?;
        }
        self.wreg(0x60, ctl | 1)?;
        self.wait(0x60, 1, 0, 600)?;
        (0..n_read).map(|i| self.rreg(0x67 + i)).collect()
    }

    pub fn jedec_id(&self) -> Result<[u8; 3]> {
        let v = self.spi_common(0x9F, None, 3)?;
        Ok([v[0], v[1], v[2]])
    }

    /// Hardware CRC-8 of flash[start .. start+len] (RTK_ReadCRC).
    pub fn crc(&self, start: u32, len: u32) -> Result<u8> {
        let end = flash_range_end(start, len)? - 1;
        for (r, v) in [
            (0x64, start >> 16),
            (0x65, start >> 8),
            (0x66, start),
            (0x72, end >> 16),
            (0x73, end >> 8),
            (0x74, end),
        ] {
            self.wreg(r, v as u8)?;
        }
        let v = self.rreg(0x6F)?;
        self.wreg(0x6F, v | 0x04)?;
        self.wait(0x6F, 0x04, 0, 600)?;
        self.rreg(0x75)
    }

    /// Read flash bytes (SPI READ 0x03, 3 bytes per common command) and cross-check the result against
    /// the hardware CRC engine before returning. This is the only read path that works: the hub needs a
    /// handshake per op, and the chip has no single-register streaming FIFO, so there is no faster route.
    pub fn read_flash(
        &self,
        start: u32,
        len: u32,
        progress: &mut dyn FnMut(u32),
    ) -> Result<Vec<u8>> {
        let hw = self.crc(start, len)?;
        let mut out = Vec::with_capacity(len as usize);
        while (out.len() as u32) < len {
            let n = (len - out.len() as u32).min(3) as u8;
            out.extend(self.spi_common(0x03, Some(start + out.len() as u32), n)?);
            progress(out.len() as u32);
        }
        out.truncate(len as usize);
        let sw = crate::crc8::crc8(&out);
        if hw != sw {
            let _ = std::fs::write("awtool-failed-read.bin", &out);
            return Err(format!(
                "read verification failed: hardware CRC {hw:02x}, data CRC {sw:02x} (data saved to awtool-failed-read.bin)"
            ));
        }
        Ok(out)
    }

    // ---------------- flashing (destructive; only runs when armed) ----------------

    fn set_op_addr(&self, block: u8, hi: u8, lo: u8) -> Result<()> {
        self.wreg(0x64, block)?;
        self.wreg(0x65, hi)?;
        self.wreg(0x66, lo)
    }

    /// Mirror of Dell's Rtd2717.RTK_DisableFlashHardwareWriteProtect (config {0x10,0x0A}, out {_,0xE3}).
    fn disable_write_protect(&self) -> Result<()> {
        self.wreg(0xF4, 0x9F)?;
        self.wreg(0xF5, 0x10)?;
        self.wreg(0xF4, 0x0A)?;
        let v = self.rreg(0xF5)?;
        self.wreg(0xF4, 0x9F)?;
        self.wreg(0xF5, 0x10)?;
        self.wreg(0xF4, 0x0A)?;
        self.wreg(0xF5, (v & 0xF8) | 0x01)?;
        let v = self.rreg(0xE3)?;
        self.wreg(0xE3, (v & 0xFB) | 0x04)
    }

    /// Erase one 64 KB block, then blank-check it via the hardware CRC (0xDE = erased). Mirror of RTK_BlockErase.
    fn block_erase(&self, block: u8) -> Result<()> {
        self.require_writable((block as u32) << 16, 0x10000)?;
        self.set_op_addr(block, 0, 0)?;
        self.wreg(0x60, 0xB8)?;
        self.wreg(0x61, 0xD8)?;
        self.wreg(0x60, 0xB9)?;
        self.wait(0x60, 0x01, 0, 600)?;
        match self.crc((block as u32) << 16, 0x10000)? {
            0xDE => Ok(()),
            c => Err(format!(
                "block {block:#04x} not blank after erase (crc {c:02x})"
            )),
        }
    }

    /// Program up to a page of bytes at (block:addr16). Mirrors RTK_SerialFlashWriteCode /
    /// RTK_FlashWriteCompleteCode exactly: reg 0x71 = (byte count − 1), which is how Dell sets it in every
    /// case (0xFF for 256 bytes, 0x00 for 1 byte). The 0x70 burst goes through the authenticated write path.
    fn program(&self, block: u8, addr16: u16, data: &[u8]) -> Result<()> {
        let addr = ((block as u32) << 16) | u32::from(addr16);
        self.require_writable(addr, data.len())?;
        if data.len() > 64 || (addr16 as usize & 0xFF) + data.len() > 256 {
            return Err("program requires 1..=64 bytes within a single 256-byte page".into());
        }
        self.wreg(0x6D, 2)?;
        self.wreg(0x71, (data.len() - 1) as u8)?;
        self.set_op_addr(block, (addr16 >> 8) as u8, addr16 as u8)?;
        let mut last = String::from("write port 0x70 failed");
        for _ in 0..5 {
            match self.write(ISP, 0x70, data) {
                Ok(()) => {
                    last.clear();
                    break;
                }
                Err(e) => last = e,
            }
        }
        if !last.is_empty() {
            return Err(last);
        }
        self.wreg(0x6F, 0xA0)?;
        self.wait(0x6F, 0x20, 0, 600)
    }

    /// Write a short run of bytes (the AA/55 marker, its 0x00 invalidation, or self-test data).
    pub fn write_byte(&self, addr: u32, byte: u8) -> Result<()> {
        self.require_writable(addr, 1)?;
        self.program((addr >> 16) as u8, addr as u16, &[byte])
    }

    pub fn write_bytes(&self, addr: u32, data: &[u8]) -> Result<()> {
        self.require_writable(addr, data.len())?;
        self.program((addr >> 16) as u8, addr as u16, data)
    }

    /// Erase one 64 KB block (public wrapper for the marker block). Blank-checks via hardware CRC.
    pub fn erase_block(&self, block: u8) -> Result<()> {
        self.block_erase(block)
    }

    /// Erase one 4 KB sector (SPI opcode 0x20) and blank-check it via hardware CRC. `addr` is sector-aligned.
    pub fn sector_erase(&self, addr: u32) -> Result<()> {
        self.require_writable(addr, 0x1000)?;
        if addr & 0xFFF != 0 {
            return Err("sector erase requires a 4 KB aligned address".into());
        }
        self.set_op_addr((addr >> 16) as u8, (addr >> 8) as u8, addr as u8)?;
        self.wreg(0x60, 0xB8)?;
        self.wreg(0x61, 0x20)?; // 4 KB sector erase
        self.wreg(0x60, 0xB9)?;
        self.wait(0x60, 0x01, 0, 600)?;
        let blank = crate::crc8::crc8(&[0xFF; 0x1000]);
        match self.crc(addr, 0x1000)? {
            c if c == blank => Ok(()),
            c => Err(format!(
                "sector {addr:#08x} not blank after erase (crc {c:02x})"
            )),
        }
    }

    /// RDSR (0x05): the flash status register 1, so we can see/clear software block-protect bits.
    pub fn status_reg1(&self) -> Result<u8> {
        Ok(self.spi_common(0x05, None, 1)?[0])
    }

    /// Clear SPI status-register block-protect bits (WRSR SR1:=0), mirroring Dell's
    /// RTK_EnableFlashWriteProtection(bEnable:false). Without this a Dell-updated unit may be fully protected.
    fn clear_status_protect(&self) -> Result<()> {
        self.wreg(0x60, 0x68)?; // type-3 (WREN) write
        self.wreg(0x61, 0x01)?; // WRSR opcode
        self.wreg(0x64, 0x00)?; // SR1 = 0x00
        self.wreg(0x60, 0x69)?; // execute
        self.wait(0x60, 0x01, 0, 600)
    }

    pub fn arm(&self) -> Result<()> {
        self.armed.set(true);
        let result = self
            .disable_write_protect()
            .and_then(|_| self.clear_status_protect());
        if result.is_err() {
            self.armed.set(false);
        }
        result
    }

    fn require_writable(&self, start: u32, len: usize) -> Result<()> {
        if !self.armed.get() {
            return Err("erase/program called without arm()".into());
        }
        crate::flash::writable_range(start, len)
    }

    /// Flash one 64 KB block from `data` (0x10000 bytes) at 4 KB-sector granularity: a sector whose hardware
    /// CRC already matches the wanted data is skipped; otherwise it is sector-erased, programmed, and verified.
    pub fn flash_block(&self, block: u8, data: &[u8], progress: &mut dyn FnMut(u32)) -> Result<()> {
        if !self.armed.get() {
            return Err("flash_block called without arm()".into());
        }
        if data.len() != 0x10000 {
            return Err(format!(
                "flash_block expects a 64 KB block, got {:#x}",
                data.len()
            ));
        }
        self.require_writable((block as u32) << 16, data.len())?;
        for s in 0..16usize {
            let off = s * 0x1000;
            let sect = &data[off..off + 0x1000];
            let addr = ((block as u32) << 16) | off as u32;
            let want = crate::crc8::crc8(sect);
            if self.crc(addr, 0x1000)? == want {
                progress((off + 0x1000) as u32);
                continue; // sector already correct on the device — leave it untouched
            }
            self.sector_erase(addr)?;
            for (j, chunk) in sect.chunks(64).enumerate() {
                if chunk.iter().all(|&b| b == 0xFF) {
                    continue; // erased sector already reads 0xFF
                }
                self.program(block, (off + j * 64) as u16, chunk)?;
            }
            match self.crc(addr, 0x1000)? {
                c if c == want => progress((off + 0x1000) as u32),
                c => {
                    return Err(format!(
                        "sector {addr:#08x} verify failed: crc {c:02x}, expected {want:02x}"
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn close(self) {} // Drop performs cleanup on success, error and panic.
}

impl Drop for Hub {
    fn drop(&mut self) {
        if self.isp_entered.get() {
            self.reboot();
        }
        self.hid.release_list();
        self.hid.close();
    }
}

/// The ISP protocol carries 24-bit addresses; reject truncation and empty CRC ranges.
pub fn flash_range_end(start: u32, len: u32) -> Result<u32> {
    let end = start.checked_add(len).ok_or("flash range overflow")?;
    if len == 0 || end > 1 << 24 {
        return Err("flash range must be nonempty and fit the 24-bit address space".into());
    }
    Ok(end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default)]
    struct Trace {
        writes: Vec<Vec<u8>>,
        closes: usize,
        releases: usize,
    }
    struct FakeHid {
        trace: Rc<RefCell<Trace>>,
        fail_isp_setup: bool,
    }

    impl HidBackend for FakeHid {
        fn count(&self, _: u16, _: u16) -> i32 {
            1
        }
        fn release_list(&self) {
            self.trace.borrow_mut().releases += 1;
        }
        fn open(&self, _: i32) -> bool {
            true
        }
        fn close(&self) {
            self.trace.borrow_mut().closes += 1;
        }
        fn handshake(&self, _: &[u8]) -> i32 {
            0
        }
        fn write(&self, buf: &[u8], _: u8, _: u8) -> i32 {
            self.trace.borrow_mut().writes.push(buf.to_vec());
            if self.fail_isp_setup && buf.first() == Some(&0xF4) {
                -1
            } else {
                0
            }
        }
        fn read(&self, _: &[u8], out: &mut [u8], _: u8, _: u8) -> i32 {
            out.fill(0x80);
            0
        }
    }

    fn hub(trace: Rc<RefCell<Trace>>, fail_isp_setup: bool) -> Hub {
        Hub {
            hid: Box::new(FakeHid {
                trace,
                fail_isp_setup,
            }),
            key: vec![],
            armed: std::cell::Cell::new(false),
            isp_entered: std::cell::Cell::new(false),
        }
    }

    #[test]
    fn partial_isp_entry_reboots_and_releases_handles_on_drop() {
        let trace = Rc::new(RefCell::new(Trace::default()));
        {
            let hub = hub(trace.clone(), true);
            assert!(hub.enter_isp().is_err());
        }
        let trace = trace.borrow();
        assert!(trace.writes.contains(&vec![0xEE, 0x82]));
        assert_eq!(trace.closes, 1);
        assert_eq!(trace.releases, 1);
    }

    #[test]
    fn invalid_ranges_and_program_requests_fail_before_any_register_write() {
        let trace = Rc::new(RefCell::new(Trace::default()));
        let hub = hub(trace.clone(), false);
        assert!(hub.crc(0, 0).is_err());
        assert!(hub.crc(0xFFFFFF, 2).is_err());
        assert!(hub.write_byte(0x400000, 0).is_err());
        hub.armed.set(true);
        assert!(hub.write_bytes(0x400000, &[]).is_err());
        assert!(hub.write_bytes(0x400000, &[0; 65]).is_err());
        assert!(hub.write_bytes(0x4000FF, &[0; 2]).is_err());
        assert!(hub.sector_erase(0x400001).is_err());
        assert!(hub.erase_block(0).is_err());
        assert!(hub.write_byte(0x1400000, 0).is_err());
        assert!(trace.borrow().writes.is_empty());
    }
}
