//! Transport abstraction over the Realtek hub's HID->I2C bridge.
//!
//! [`crate::rthid::RtHid`] implements [`HidBackend`] over Dell's `RtHIDlib.dll`; the trait keeps
//! [`crate::isp::Hub`] testable against a fake transport.
//!
//! The method set mirrors the DLL's exported C API exactly (global-state design, hence `&self`):
//! `count` enumerates matching hubs, `open` selects one, `handshake` runs the rolling challenge-
//! response, and `write`/`read` perform one authenticated I2C transaction each.

pub trait HidBackend {
    /// Number of connected hubs matching (vid, pid); also (re)builds the internal candidate list.
    fn count(&self, vid: u16, pid: u16) -> i32;
    /// Release the candidate list built by [`count`](Self::count).
    fn release_list(&self);
    /// Open the `index`-th candidate from the last [`count`](Self::count). Returns true on success.
    fn open(&self, index: i32) -> bool;
    /// Close the currently-open hub.
    fn close(&self);
    /// Run one rolling OEM handshake with `key`. Returns 0 on success (same convention as the DLL).
    fn handshake(&self, key: &[u8]) -> i32;
    /// One I2C write: `buf` is sub-address + data, `slave8` the 8-bit (shifted) address, `speed` the bus speed.
    fn write(&self, buf: &[u8], slave8: u8, speed: u8) -> i32;
    /// One I2C read: write `w` (sub-address bytes) then read `r.len()` bytes back into `r`.
    fn read(&self, w: &[u8], r: &mut [u8], slave8: u8, speed: u8) -> i32;
}
