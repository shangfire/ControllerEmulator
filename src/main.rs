/*
 * @Author: shanghanjin
 * @Date: 2025-01-22 17:32:42
 * @LastEditTime: 2025-02-23 18:08:51
 * @FilePath: \ControllerEmulator\src\main.rs
 * @Description: 
 */
use mlua::prelude::*;
use std::fs;
use std::io;
use std::env;
use std::ffi::c_void;
use windows::Win32::System::Services::*;
use windows::core::w;
use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::System::IO::*;
use windows::Win32::Storage::FileSystem::*;

/**
 * @description: 加载驱动程序
 * @param {*} Result 返回值
 * @return {*}
 */
fn load_driver() -> Result<(), std::io::Error> {
    unsafe {
        // 获取当前可执行文件路径
        let exe_path = env::current_exe()?;
        let exe_dir = exe_path.parent().unwrap();
        
        // 构造驱动程序的完整路径
        let driver_path = exe_dir.join("driver\\VirtualKMDriver.sys");
        let wide_driver_path: Vec<u16> = driver_path.to_str().unwrap().encode_utf16().chain(Some(0)).collect();

        // 打开服务控制管理器
        let scm = OpenSCManagerW(None, None, SC_MANAGER_CREATE_SERVICE)?;
        if scm.is_invalid() {
            return Err(std::io::Error::last_os_error());
        }

        // 检查服务是否存在
        let mut service = OpenServiceW(scm, w!("VirtualKMDriver"), SERVICE_QUERY_STATUS)?;
        
        // 如果服务不存在，创建新服务
        if service.is_invalid() {
            service = CreateServiceW(
                scm,
                w!("VirtualKMDriver"),    // 服务名称
                w!("Virtual Keyboard Mouse Driver"),
                SERVICE_ALL_ACCESS,
                SERVICE_KERNEL_DRIVER,
                SERVICE_DEMAND_START,
                SERVICE_ERROR_NORMAL,
                PCWSTR(wide_driver_path.as_ptr()),
                None,
                None,
                None,
                None,
                None,
            )?;

            if service.is_invalid() {
                return Err(std::io::Error::last_os_error());
            }
        }

        // 检查服务是否已经在运行
        let mut status: SERVICE_STATUS_PROCESS = std::mem::zeroed();
        let mut bytes_needed: u32 = 0;
        let buffer = std::slice::from_raw_parts_mut(
                &mut status as *mut _ as *mut u8, 
                std::mem::size_of::<SERVICE_STATUS_PROCESS>()
            );

        let success = QueryServiceStatusEx(
            service,
            SC_STATUS_PROCESS_INFO,
            Some(buffer),
            &mut bytes_needed
        );

        if success.is_err() {
            if status.dwCurrentState == SERVICE_RUNNING {
                println!("驱动已在运行");
                return Ok(()); // 直接返回，不再启动
            }
        }

        // 启动服务
        let success = StartServiceW(service, None);
        if success.is_err() {
            return Err(std::io::Error::last_os_error());
        }

        return Ok(())
    }
}

const IOCTL_SEND_KEY: u32 = 0x80002000; // 自定义 IOCTL 码

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct KeyInput {
    key_code: u8,
    key_state: u8, // 0 = 按下, 1 = 释放
}

struct DriverConnection {
    device: HANDLE,
}

impl DriverConnection {
    /// 打开驱动设备
    fn new() -> Result<Self, std::io::Error> {
        unsafe {
            let device_path = w!("\\\\.\\VirtualKMDriver");
            let device = CreateFileW(
                device_path,
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                HANDLE::default(),
            )?;

            if device.is_invalid() {
                return Err(std::io::Error::last_os_error());
            }

            Ok(Self { device })
        }
    }

    /// 发送按键数据
    fn send_key(&self, key_code: u8, key_state: u8) -> Result<(), std::io::Error> {
        unsafe {
            let input: KeyInput = KeyInput { key_code, key_state };
            let mut bytes_returned = 0;

            let success = DeviceIoControl(
                self.device,
                IOCTL_SEND_KEY,
                Some(&input as *const _ as *mut c_void),
                std::mem::size_of::<KeyInput>() as u32,
                None,
                0,
                Some(&mut bytes_returned),
                None,
            );

            if !success.is_err() {
                println!("按键消息已发送: key_code={}, key_state={}", key_code, key_state);
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        }
    }
}

impl Drop for DriverConnection {
    fn drop(&mut self) {
        unsafe {
            if let Err(e) = CloseHandle(self.device) {
                eprintln!("Failed to close handle: {:?}", e);
            }
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 加载驱动程序
    if let Err(e) = load_driver() {
        println!("load driver failed: {}", e);
        return Ok(());
    }

    //  打开驱动设备
    let driver = DriverConnection::new()?;
    println!("driver connected");

    // 创建 Lua 实例
    let lua = Lua::new();
    let scripts_dir = "scripts";

    // 读取目录下的所有.lua文件
    let mut scripts = vec![];
    if let Ok(entries) = fs::read_dir(scripts_dir) {
        for (id, entry) in entries.filter_map(Result::ok).enumerate() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("lua") {
                scripts.push((id, path.clone()));
            }
        }
    }

    if scripts.is_empty() {
        println!("can't find any lua script in {}", scripts_dir);
        return Ok(());
    }

    // 展示可用的lua脚本
    println!("usable scripts:");
    for (id, path) in &scripts {
        println!("[{}]: {}", id, path.display());
    }

    // 读取用户输入的脚本编号
    let mut input = String::new();
    println!("please select a script to run: ");
    io::stdin().read_line(&mut input).expect("read line failed");
    let input = input.trim();
    let selected_id:usize = match input.parse() {
        Ok(id) if id < scripts.len() => id,
        _ => {
            println!("invalid script id: {}", input);
            return Ok(());
        }
    };

    // 定义回调函数并注册到 Lua 环境
    let callback = lua.create_function(move |_, ()| {
        println!("Lua called callback!");
        driver.send_key(0x1E, 0)?; // A
        Ok(())
    })?;
    lua.globals().set("rust_callback", callback)?;

    // 读取并执行lua脚本
    let script_path = &scripts[selected_id].1;
    let script_content = fs::read_to_string(script_path).expect("读取脚本失败");

    lua.load(&script_content).exec()?;
    println!("executed script: {}", script_path.display());

    Ok(())
}
