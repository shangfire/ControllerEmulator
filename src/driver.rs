use std::{ffi::c_void, io};
use windows::{
    core::w,
    Win32::{Foundation::*, Storage::FileSystem::*, System::IO::*},
};

// 与 Public.h 的 METHOD_BUFFERED 协议一致；显式序列化避免结构体填充字节。
pub const KEYBOARD: u32 = 0x80002004;
pub const MOUSE: u32 = 0x80002008;
pub const ABS_MOUSE: u32 = 0x8000200c;
pub const RESET: u32 = 0x80002010;
pub const STATUS: u32 = 0x80002014;
pub struct Driver(HANDLE);
impl Driver {
    pub fn open() -> io::Result<Self> {
        unsafe {
            let handle = CreateFileW(
                w!("\\\\.\\KMDriver"),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )?;
            let driver = Self(handle);
            let mut version = [0u8; 4];
            let mut returned = 0;
            DeviceIoControl(
                handle,
                STATUS,
                None,
                0,
                Some(version.as_mut_ptr().cast()),
                4,
                Some(&mut returned),
                None,
            )?;
            if returned != 4 || u32::from_le_bytes(version) != 1 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "驱动协议版本不匹配",
                ));
            }
            Ok(driver)
        }
    }
    pub fn send(&self, code: u32, data: &[u8]) -> io::Result<()> {
        unsafe {
            let mut returned = 0;
            DeviceIoControl(
                self.0,
                code,
                if data.is_empty() {
                    None
                } else {
                    Some(data.as_ptr() as *const c_void)
                },
                data.len() as u32,
                None,
                0,
                Some(&mut returned),
                None,
            )?;
        }
        Ok(())
    }
}
impl Drop for Driver {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
pub fn mouse_bytes(x: u16, y: u16, buttons: u16) -> [u8; 6] {
    let [a, b] = x.to_le_bytes();
    let [c, d] = y.to_le_bytes();
    let [e, f] = buttons.to_le_bytes();
    [a, b, c, d, e, f]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_layout() {
        assert_eq!(
            mouse_bytes((-127i16) as u16, 32768, 5),
            [129, 255, 0, 128, 5, 0]
        );
        assert_eq!(KEYBOARD, (0x8000 << 16) | (0x801 << 2));
    }
}
