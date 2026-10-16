//! Read-only DDC/CI through the Windows monitor configuration API (dxva2.dll): no admin rights, no writes.

use std::ffi::c_void;
use std::ptr::null_mut;

#[repr(C)]
pub struct PhysicalMonitor {
    pub handle: *mut c_void,
    pub description: [u16; 128],
}

type MonitorEnumProc =
    unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, isize) -> i32;

#[link(name = "user32")]
unsafe extern "system" {
    fn EnumDisplayMonitors(
        hdc: *mut c_void,
        clip: *const c_void,
        cb: MonitorEnumProc,
        data: isize,
    ) -> i32;
}

#[link(name = "dxva2")]
unsafe extern "system" {
    fn GetNumberOfPhysicalMonitorsFromHMONITOR(hmon: *mut c_void, n: *mut u32) -> i32;
    fn GetPhysicalMonitorsFromHMONITOR(hmon: *mut c_void, n: u32, out: *mut PhysicalMonitor)
    -> i32;
    fn DestroyPhysicalMonitors(n: u32, mons: *const PhysicalMonitor) -> i32;
    fn GetVCPFeatureAndVCPFeatureReply(
        h: *mut c_void,
        code: u8,
        kind: *mut u32,
        cur: *mut u32,
        max: *mut u32,
    ) -> i32;
    fn GetCapabilitiesStringLength(h: *mut c_void, len: *mut u32) -> i32;
    fn CapabilitiesRequestAndCapabilitiesReply(h: *mut c_void, buf: *mut u8, len: u32) -> i32;
}

unsafe extern "system" fn on_monitor(
    hmon: *mut c_void,
    _hdc: *mut c_void,
    _rect: *mut c_void,
    data: isize,
) -> i32 {
    // `data` is the `*mut Vec<PhysicalMonitor>` passed by `Monitors::enumerate`, alive for the whole call.
    let list = unsafe { &mut *(data as *mut Vec<PhysicalMonitor>) };
    let mut n = 0u32;
    unsafe {
        if GetNumberOfPhysicalMonitorsFromHMONITOR(hmon, &mut n) != 0 && n > 0 {
            let mut v: Vec<PhysicalMonitor> = (0..n)
                .map(|_| PhysicalMonitor {
                    handle: null_mut(),
                    description: [0; 128],
                })
                .collect();
            if GetPhysicalMonitorsFromHMONITOR(hmon, n, v.as_mut_ptr()) != 0 {
                list.append(&mut v);
            }
        }
    }
    1
}

/// All physical monitors of the desktop; handles are destroyed on drop.
pub struct Monitors(Vec<PhysicalMonitor>);

impl Monitors {
    pub fn enumerate() -> Result<Monitors, String> {
        let mut list: Vec<PhysicalMonitor> = Vec::new();
        unsafe {
            EnumDisplayMonitors(
                null_mut(),
                std::ptr::null(),
                on_monitor,
                &mut list as *mut _ as isize,
            )
        };
        if list.is_empty() {
            return Err("no physical monitor found (dxva2)".into());
        }
        Ok(Monitors(list))
    }

    pub fn iter(&self) -> impl Iterator<Item = &PhysicalMonitor> {
        self.0.iter()
    }
}

impl Drop for Monitors {
    fn drop(&mut self) {
        unsafe { DestroyPhysicalMonitors(self.0.len() as u32, self.0.as_ptr()) };
    }
}

impl PhysicalMonitor {
    pub fn name(&self) -> String {
        let end = self.description.iter().position(|&c| c == 0).unwrap_or(128);
        String::from_utf16_lossy(&self.description[..end])
    }

    /// The raw MCCS capabilities string ("(prot(monitor)type(lcd)...vcp(...))").
    pub fn capabilities(&self) -> Result<String, String> {
        let mut len = 0u32;
        unsafe {
            if GetCapabilitiesStringLength(self.handle, &mut len) == 0 || len == 0 {
                return Err("GetCapabilitiesStringLength failed (no DDC/CI reply)".into());
            }
            let mut buf = vec![0u8; len as usize];
            if CapabilitiesRequestAndCapabilitiesReply(self.handle, buf.as_mut_ptr(), len) == 0 {
                return Err("CapabilitiesRequestAndCapabilitiesReply failed".into());
            }
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            Ok(String::from_utf8_lossy(&buf[..end]).into_owned())
        }
    }

    /// Read one VCP feature: (kind: 0 = set parameter, 2 = momentary, current, maximum).
    pub fn vcp(&self, code: u8) -> Option<(u32, u32, u32)> {
        let (mut kind, mut cur, mut max) = (0u32, 0u32, 0u32);
        let ok = unsafe {
            GetVCPFeatureAndVCPFeatureReply(self.handle, code, &mut kind, &mut cur, &mut max)
        };
        (ok != 0).then_some((kind, cur, max))
    }
}
