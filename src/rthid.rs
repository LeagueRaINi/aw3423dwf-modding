//! Minimal binding to Dell's RtHIDlib.dll (Realtek USB hub HID -> I2C bridge), loaded at runtime.
//! The DLL and its MFC/VC runtime dependencies ship with Dell's updater; pass that folder with --updater-dir.

use std::ffi::c_void;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetDllDirectoryW(path: *const u16) -> i32;
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    fn FreeLibrary(module: *mut c_void) -> i32;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

type GetMulti = unsafe extern "C" fn(u16, u16) -> i32;
type PutMulti = unsafe extern "C" fn();
type OpenMulti = unsafe extern "C" fn(i32) -> i32;
type CloseHid = unsafe extern "C" fn();
type Handshake = unsafe extern "C" fn(*const u8, u8) -> i32;
type I2cWrite = unsafe extern "C" fn(*const u8, u8, u8, u8) -> i32;
type I2cRead2 = unsafe extern "C" fn(*const u8, u8, *mut u8, u8, u8, u8, u32) -> i32;

pub struct RtHid {
    _library: Library,
    get_multi: GetMulti,
    put_multi: PutMulti,
    open_multi: OpenMulti,
    close_hid: CloseHid,
    handshake: Handshake,
    i2c_write: I2cWrite,
    i2c_read2: I2cRead2,
}

/// Keep exported function pointers alive and unload on every error path.
struct Library(*mut c_void);

impl Drop for Library {
    fn drop(&mut self) {
        // SAFETY: this handle comes from a successful LoadLibraryW and is owned here.
        unsafe {
            FreeLibrary(self.0);
        }
    }
}

macro_rules! sym {
    ($m:expr, $name:literal, $ty:ty) => {{
        let p = GetProcAddress($m, concat!($name, "\0").as_ptr());
        if p.is_null() {
            return Err(format!("RtHIDlib.dll has no export {}", $name));
        }
        // SAFETY: the named export uses this exact signature in Dell's 32-bit DLL.
        std::mem::transmute::<*mut c_void, $ty>(p)
    }};
}

impl RtHid {
    pub fn load(dll_dir: &str) -> Result<Self, String> {
        if cfg!(target_pointer_width = "64") {
            return Err("Dell's RtHIDlib.dll is 32-bit; build for i686-pc-windows-msvc".into());
        }
        unsafe {
            if SetDllDirectoryW(wide(dll_dir).as_ptr()) == 0 {
                return Err(format!("could not set DLL directory: {dll_dir}"));
            }
            let m = LoadLibraryW(wide(&format!("{dll_dir}\\RtHIDlib.dll")).as_ptr());
            if m.is_null() {
                return Err(format!("could not load {dll_dir}\\RtHIDlib.dll"));
            }
            let library = Library(m);
            Ok(RtHid {
                get_multi: sym!(m, "Get_mutiHID", GetMulti),
                put_multi: sym!(m, "Put_mutiHID", PutMulti),
                open_multi: sym!(m, "Open_mutiHID", OpenMulti),
                close_hid: sym!(m, "Close_HID", CloseHid),
                handshake: sym!(m, "HID_handshakeauth2", Handshake),
                i2c_write: sym!(m, "HID_i2cwrite", I2cWrite),
                i2c_read2: sym!(m, "HID_i2cread2", I2cRead2),
                _library: library,
            })
        }
    }
}

impl crate::hid::HidBackend for RtHid {
    fn count(&self, vid: u16, pid: u16) -> i32 {
        unsafe { (self.get_multi)(vid, pid) }
    }
    fn release_list(&self) {
        unsafe { (self.put_multi)() }
    }
    fn open(&self, index: i32) -> bool {
        unsafe { (self.open_multi)(index) == 0 }
    }
    fn close(&self) {
        unsafe { (self.close_hid)() }
    }
    fn handshake(&self, key: &[u8]) -> i32 {
        if key.len() > 100 {
            return 1;
        }
        unsafe { (self.handshake)(key.as_ptr(), key.len() as u8) }
    }
    /// `slave8` is the 8-bit (shifted) address, `buf` starts with the sub-address.
    fn write(&self, buf: &[u8], slave8: u8, speed: u8) -> i32 {
        if buf.len() > 128 {
            return 1;
        }
        unsafe { (self.i2c_write)(buf.as_ptr(), buf.len() as u8, slave8, speed) }
    }
    fn read(&self, w: &[u8], r: &mut [u8], slave8: u8, speed: u8) -> i32 {
        if w.len() > 5 || r.len() > u8::MAX as usize {
            return 1;
        }
        let wp = if w.is_empty() {
            std::ptr::null()
        } else {
            w.as_ptr()
        };
        unsafe {
            (self.i2c_read2)(
                wp,
                w.len() as u8,
                r.as_mut_ptr(),
                r.len() as u8,
                slave8,
                speed,
                100,
            )
        }
    }
}
