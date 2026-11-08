use crate::driver::{self, Driver};
use mlua::{HookTriggers, Lua, MultiValue, Value};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::SyncSender,
        Arc,
    },
    time::{Duration, Instant},
};

#[derive(Debug)]
pub enum Event {
    Log(String),
    Finished(Result<(), String>),
}
#[derive(Clone, Copy)]
pub enum Mode {
    Test,
    Driver,
}

struct Execution {
    driver: Option<Driver>,
    log: SyncSender<Event>,
    stop: Arc<AtomicBool>,
    started: Instant,
    count: usize,
    keys: [bool; 256],
    buttons: u16,
}
impl Execution {
    fn check(&self) -> mlua::Result<()> {
        if self.stop.load(Ordering::Relaxed) {
            return Err(mlua::Error::RuntimeError("已停止".into()));
        }
        if self.started.elapsed() > Duration::from_secs(300) {
            return Err(mlua::Error::RuntimeError(
                "超过单次运行时限（5 分钟）".into(),
            ));
        }
        Ok(())
    }
    fn command(&mut self, code: u32, bytes: &[u8], description: String) -> mlua::Result<()> {
        self.check()?;
        self.count += 1;
        if self.count > 100_000 {
            return Err(mlua::Error::RuntimeError(
                "超过单次命令上限（100000）".into(),
            ));
        }
        if let Some(driver) = &self.driver {
            driver.send(code, bytes).map_err(mlua::Error::external)?;
        }
        let _ = self.log.try_send(Event::Log(description));
        Ok(())
    }
    fn key(&mut self, key: u8, down: bool) -> mlua::Result<()> {
        self.command(
            driver::KEYBOARD,
            &[key, down as u8],
            format!("键盘 0x{key:02X} {}", if down { "按下" } else { "释放" }),
        )?;
        self.keys[key as usize] = down;
        Ok(())
    }
    fn mouse(&mut self, x: u16, y: u16, buttons: u16, absolute: bool) -> mlua::Result<()> {
        if buttons > 7 {
            return Err(mlua::Error::RuntimeError("鼠标按键掩码必须为 0..7".into()));
        }
        self.command(
            if absolute {
                driver::ABS_MOUSE
            } else {
                driver::MOUSE
            },
            &driver::mouse_bytes(x, y, buttons),
            if absolute {
                format!("鼠标绝对位置 ({x}, {y})，按键 {buttons}")
            } else {
                format!("鼠标相对移动 ({}, {})，按键 {buttons}", x as i16, y as i16)
            },
        )?;
        self.buttons = buttons;
        Ok(())
    }
    fn release(&mut self) -> Result<(), String> {
        if let Some(driver) = &self.driver {
            driver
                .send(driver::RESET, &[])
                .map_err(|e| format!("释放输入失败：{e}"))?;
        }
        self.keys.fill(false);
        self.buttons = 0;
        let _ = self.log.try_send(Event::Log("释放全部键鼠按键".into()));
        Ok(())
    }
}

pub fn key_code(value: Value) -> mlua::Result<u8> {
    let numeric = match value {
        Value::Integer(n) if (1..=255).contains(&n) => Some(n as u8),
        Value::String(s) => {
            let name = s.to_str()?.to_uppercase();
            match name.as_str() {
                "CTRL" | "LCTRL" => Some(0xA2),
                "RCTRL" => Some(0xA3),
                "SHIFT" | "LSHIFT" => Some(0xA0),
                "RSHIFT" => Some(0xA1),
                "ALT" | "LALT" => Some(0xA4),
                "RALT" => Some(0xA5),
                "WIN" | "LWIN" => Some(0x5B),
                "RWIN" => Some(0x5C),
                "ENTER" => Some(13),
                "ESC" | "ESCAPE" => Some(27),
                "SPACE" => Some(32),
                "TAB" => Some(9),
                "BACKSPACE" => Some(8),
                "DELETE" => Some(46),
                "LEFT" => Some(37),
                "UP" => Some(38),
                "RIGHT" => Some(39),
                "DOWN" => Some(40),
                "HOME" => Some(36),
                "END" => Some(35),
                "PAGEUP" => Some(33),
                "PAGEDOWN" => Some(34),
                _ if name.len() == 1 && name.as_bytes()[0].is_ascii_alphanumeric() => {
                    Some(name.as_bytes()[0])
                }
                _ if name.starts_with('F') => name[1..]
                    .parse::<u8>()
                    .ok()
                    .filter(|n| (1..=12).contains(n))
                    .map(|n| 0x6f + n),
                _ => None,
            }
        }
        _ => None,
    };
    numeric
        .filter(|n| supported_key(*n))
        .ok_or_else(|| mlua::Error::RuntimeError("不支持的键名或虚拟键码".into()))
}
fn supported_key(n: u8) -> bool {
    matches!(n, 8|9|13|20|27|32..=40|46|48..=57|65..=90|91|92|112..=123|160..=165|186..=192|219..=222)
}

fn wait(state: &Rc<RefCell<Execution>>, ms: u64) -> mlua::Result<()> {
    if ms > 300_000 {
        return Err(mlua::Error::RuntimeError(
            "等待时间必须为 0..300000 毫秒".into(),
        ));
    }
    let until = Instant::now() + Duration::from_millis(ms);
    loop {
        state.borrow().check()?;
        let remaining = until.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(());
        }
        std::thread::sleep(remaining.min(Duration::from_millis(10)));
    }
}

pub fn syntax(source: &str) -> Result<(), String> {
    Lua::new()
        .load(source)
        .set_name("编辑器")
        .into_function()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn execute(
    source: String,
    name: String,
    mode: Mode,
    stop: Arc<AtomicBool>,
    log: SyncSender<Event>,
) -> Result<(), String> {
    let driver = match mode {
        Mode::Test => None,
        Mode::Driver => Some(
            Driver::open()
                .map_err(|e| format!("连接驱动失败：{e}。请先安装并启动 KMDriver 设备。"))?,
        ),
    };
    let state = Rc::new(RefCell::new(Execution {
        driver,
        log,
        stop,
        started: Instant::now(),
        count: 0,
        keys: [false; 256],
        buttons: 0,
    }));
    let result = run_lua(&source, &name, state.clone()).map_err(|e| e.to_string());
    let released = state.borrow_mut().release();
    match (result, released) {
        (Err(error), Err(cleanup)) => Err(format!("{error}\n{cleanup}")),
        (Err(error), _) | (_, Err(error)) => Err(error),
        _ => Ok(()),
    }
}

fn run_lua(source: &str, name: &str, state: Rc<RefCell<Execution>>) -> mlua::Result<()> {
    let lua = Lua::new();
    lua.set_memory_limit(32 * 1024 * 1024)?;
    let hook_state = state.clone();
    lua.set_hook(
        HookTriggers {
            every_nth_instruction: Some(1000),
            ..Default::default()
        },
        move |_, _| {
            hook_state.borrow().check()?;
            Ok(mlua::VmState::Continue)
        },
    )?;
    let globals = lua.globals();
    // 脚本只开放计算、字符串、表和键鼠 API，避免阻塞式系统调用绕过停止按钮。
    for name in [
        "os",
        "io",
        "package",
        "require",
        "dofile",
        "loadfile",
        "debug",
        "coroutine",
        "newproxy",
    ] {
        globals.set(name, Value::Nil)?;
    }
    let print_state = state.clone();
    globals.set(
        "print",
        lua.create_function(move |lua, args: MultiValue| {
            let tostring: mlua::Function = lua.globals().get("tostring")?;
            let mut parts = Vec::new();
            for value in args {
                parts.push(tostring.call::<String>(value)?);
            }
            print_state.borrow().check()?;
            let _ = print_state
                .borrow()
                .log
                .try_send(Event::Log(parts.join("\t").chars().take(4096).collect()));
            Ok(())
        })?,
    )?;
    let api = lua.create_table()?;
    for (name, down) in [("key_down", true), ("key_up", false)] {
        let s = state.clone();
        api.set(
            name,
            lua.create_function(move |_, value: Value| s.borrow_mut().key(key_code(value)?, down))?,
        )?;
    }
    let s = state.clone();
    api.set(
        "key_press",
        lua.create_function(move |_, (value, ms): (Value, Option<u64>)| {
            let key = key_code(value)?;
            s.borrow_mut().key(key, true)?;
            wait(&s, ms.unwrap_or(30))?;
            s.borrow_mut().key(key, false)
        })?,
    )?;
    let s = state.clone();
    api.set(
        "sleep",
        lua.create_function(move |_, ms: u64| wait(&s, ms))?,
    )?;
    let s = state.clone();
    api.set(
        "mouse_move",
        lua.create_function(move |_, (x, y): (i16, i16)| {
            if !(-127..=127).contains(&x) || !(-127..=127).contains(&y) {
                return Err(mlua::Error::RuntimeError(
                    "单次相对位移必须为 -127..127".into(),
                ));
            }
            let buttons = s.borrow().buttons;
            s.borrow_mut().mouse(x as u16, y as u16, buttons, false)
        })?,
    )?;
    let s = state.clone();
    api.set(
        "mouse_to",
        lua.create_function(move |_, (x, y): (u16, u16)| {
            let buttons = s.borrow().buttons;
            s.borrow_mut().mouse(x, y, buttons, true)
        })?,
    )?;
    for (name, down) in [("mouse_down", true), ("mouse_up", false)] {
        let s = state.clone();
        api.set(
            name,
            lua.create_function(move |_, button: String| {
                let bit = button_bit(&button)?;
                let buttons = s.borrow().buttons;
                s.borrow_mut().mouse(
                    0,
                    0,
                    if down { buttons | bit } else { buttons & !bit },
                    false,
                )
            })?,
        )?;
    }
    let s = state.clone();
    api.set(
        "mouse_click",
        lua.create_function(move |_, button: Option<String>| {
            let bit = button_bit(button.as_deref().unwrap_or("left"))?;
            let buttons = s.borrow().buttons;
            s.borrow_mut().mouse(0, 0, buttons | bit, false)?;
            wait(&s, 30)?;
            s.borrow_mut().mouse(0, 0, buttons, false)
        })?,
    )?;
    let s = state.clone();
    api.set(
        "release_all",
        lua.create_function(move |_, ()| s.borrow_mut().release().map_err(mlua::Error::external))?,
    )?;
    globals.set("km", api)?;
    // 兼容原有示例入口。
    let s = state;
    globals.set(
        "rust_callback",
        lua.create_function(move |_, ()| {
            s.borrow_mut().key(0x41, true)?;
            s.borrow_mut().key(0x41, false)
        })?,
    )?;
    let result = lua.load(source).set_name(name).exec();
    result
}
fn button_bit(name: &str) -> mlua::Result<u16> {
    match name {
        "left" => Ok(1),
        "right" => Ok(2),
        "middle" => Ok(4),
        _ => Err(mlua::Error::RuntimeError(
            "鼠标按键应为 left / right / middle".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn test(source: &str, stopped: bool) -> (Result<(), String>, Vec<Event>) {
        let (tx, rx) = std::sync::mpsc::sync_channel(100);
        let result = execute(
            source.into(),
            "test".into(),
            Mode::Test,
            Arc::new(AtomicBool::new(stopped)),
            tx,
        );
        (result, rx.try_iter().collect())
    }
    #[test]
    fn script_commands_and_cleanup() {
        let (result,events) = test("km.key_down('CTRL'); km.key_press('A',0); km.mouse_to(32768,32768); km.mouse_click('left'); print('完成')",false);
        assert!(result.is_ok(), "{result:?}");
        assert!(events
            .iter()
            .any(|e| matches!(e, Event::Log(s) if s.contains("0xA2"))));
        assert!(matches!(events.last(),Some(Event::Log(s)) if s.contains("释放全部")));
    }
    #[test]
    fn invalid_input_and_syntax() {
        for source in [
            "km.key_press('unknown')",
            "km.mouse_move(128,0)",
            "km.mouse_click('bad')",
            "km.sleep(-1)",
            "os.execute('echo test')",
        ] {
            assert!(test(source, false).0.is_err(), "{source}");
        }
        assert!(syntax("local =").is_err());
    }
    #[test]
    fn infinite_loop_is_cancellable() {
        assert!(test("while true do end", true)
            .0
            .unwrap_err()
            .contains("已停止"));
    }
    #[test]
    fn stop_during_loop_and_sleep() {
        for source in ["while true do end", "km.sleep(300000)"] {
            let (tx, _rx) = std::sync::mpsc::sync_channel(100);
            let stop = Arc::new(AtomicBool::new(false));
            let signal = stop.clone();
            let worker = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(30));
                signal.store(true, Ordering::Relaxed);
            });
            let start = Instant::now();
            let result = execute(source.into(), "stop_test".into(), Mode::Test, stop, tx);
            worker.join().unwrap();
            assert!(result.unwrap_err().contains("已停止"));
            assert!(start.elapsed() < Duration::from_secs(2));
        }
    }
    #[test]
    fn memory_limit_reports_error() {
        let (result, _) = test("local s = string.rep('x', 64*1024*1024)", false);
        assert!(result.is_err());
    }
    #[test]
    fn error_releases_keys() {
        let (result, events) = test("km.key_down('A'); error('故障')", false);
        assert!(result.is_err());
        assert!(matches!(events.last(),Some(Event::Log(s)) if s.contains("释放全部")));
    }
}
