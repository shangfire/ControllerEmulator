use crate::{
    driver::Driver,
    runtime::{self, Event, Mode},
    scripts::Scripts,
};
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
        Arc,
    },
    thread::JoinHandle,
};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{Controls::*, WindowsAndMessaging::*},
    },
};

const LIST: usize = 101;
const NAME: usize = 102;
const EDITOR: usize = 103;
const NEW: usize = 201;
const SAVE: usize = 202;
const RENAME: usize = 203;
const DELETE: usize = 204;
const CHECK: usize = 205;
const TEST: usize = 206;
const RUN: usize = 207;
const STOP: usize = 208;
const CONNECT: usize = 209;
const CLEAR: usize = 210;
const TEMPLATE: &str = "-- KMEmulator Lua 脚本\r\n-- 模拟测试只记录命令，不产生系统输入。\r\nprint('开始')\r\nkm.key_press('A')\r\nkm.sleep(200)\r\nkm.mouse_move(50, 0)\r\nkm.mouse_click('left')\r\nprint('完成')\r\n";

struct App {
    hwnd: HWND,
    list: HWND,
    name: HWND,
    editor: HWND,
    log: HWND,
    status: HWND,
    buttons: Vec<(usize, HWND)>,
    scripts: Scripts,
    names: Vec<String>,
    selected: Option<String>,
    saved: String,
    log_text: String,
    worker: Option<JoinHandle<()>>,
    receiver: Option<Receiver<Event>>,
    stop: Arc<AtomicBool>,
    closing: bool,
}
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
unsafe fn set_text(hwnd: HWND, text: &str) {
    let _ = SetWindowTextW(hwnd, PCWSTR(wide(text).as_ptr()));
}
unsafe fn text(hwnd: HWND) -> String {
    let mut value = vec![0; GetWindowTextLengthW(hwnd) as usize + 1];
    let length = GetWindowTextW(hwnd, &mut value);
    String::from_utf16_lossy(&value[..length as usize])
}
unsafe fn control(
    parent: HWND,
    class: PCWSTR,
    title: &str,
    style: WINDOW_STYLE,
    id: usize,
) -> HWND {
    let hwnd = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        class,
        PCWSTR(wide(title).as_ptr()),
        WS_CHILD | WS_VISIBLE | style,
        0,
        0,
        1,
        1,
        parent,
        HMENU(id as isize),
        None,
        None,
    );
    SendMessageW(
        hwnd,
        WM_SETFONT,
        WPARAM(GetStockObject(DEFAULT_GUI_FONT).0 as usize),
        LPARAM(1),
    );
    hwnd
}
impl App {
    unsafe fn new(hwnd: HWND, scripts: Scripts) -> Self {
        let list = control(
            hwnd,
            w!("LISTBOX"),
            "",
            WS_VSCROLL | WS_TABSTOP | WINDOW_STYLE(LBS_NOTIFY as u32),
            LIST,
        );
        let name = control(
            hwnd,
            w!("EDIT"),
            "",
            WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            NAME,
        );
        let editor = control(
            hwnd,
            w!("EDIT"),
            "",
            WS_TABSTOP
                | WS_VSCROLL
                | WS_HSCROLL
                | WINDOW_STYLE(
                    (ES_MULTILINE | ES_AUTOVSCROLL | ES_AUTOHSCROLL | ES_WANTRETURN) as u32,
                ),
            EDITOR,
        );
        SendMessageW(editor, EM_SETLIMITTEXT, WPARAM(1024 * 1024), LPARAM(0));
        let log = control(
            hwnd,
            w!("EDIT"),
            "",
            WS_VSCROLL | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY) as u32),
            0,
        );
        SendMessageW(log, EM_SETLIMITTEXT, WPARAM(150_000), LPARAM(0));
        let status = control(
            hwnd,
            w!("STATIC"),
            "就绪 · 模拟测试无需驱动 · F8 停止",
            WINDOW_STYLE(0),
            0,
        );
        let mut buttons = Vec::new();
        for (id, label) in [
            (NEW, "新建"),
            (SAVE, "保存"),
            (RENAME, "重命名"),
            (DELETE, "删除"),
            (CHECK, "语法检查"),
            (TEST, "模拟测试"),
            (RUN, "运行到驱动"),
            (STOP, "停止 F8"),
            (CONNECT, "检测驱动"),
            (CLEAR, "清空日志"),
        ] {
            buttons.push((id, control(hwnd, w!("BUTTON"), label, WS_TABSTOP, id)));
        }
        Self {
            hwnd,
            list,
            name,
            editor,
            log,
            status,
            buttons,
            scripts,
            names: vec![],
            selected: None,
            saved: String::new(),
            log_text: String::new(),
            worker: None,
            receiver: None,
            stop: Arc::new(AtomicBool::new(false)),
            closing: false,
        }
    }
    unsafe fn layout(&self) {
        let mut rect = RECT::default();
        let _ = GetClientRect(self.hwnd, &mut rect);
        let width = rect.right.max(800);
        let height = rect.bottom.max(550);
        let _ = MoveWindow(self.list, 12, 58, 210, height - 98, true);
        let _ = MoveWindow(self.name, 234, 58, width - 246, 28, true);
        let _ = MoveWindow(self.editor, 234, 96, width - 246, height - 338, true);
        let _ = MoveWindow(self.log, 234, height - 232, width - 246, 188, true);
        let _ = MoveWindow(self.status, 12, height - 32, width - 24, 24, true);
        let mut x = 12;
        for (id, button) in &self.buttons {
            let bw = match *id {
                RUN => 100,
                CHECK | TEST | CONNECT | CLEAR | STOP => 86,
                RENAME => 70,
                _ => 58,
            };
            let _ = MoveWindow(*button, x, 12, bw, 32, true);
            x += bw + 5;
        }
    }
    unsafe fn append(&mut self, message: &str) {
        self.log_text.push_str(&message.replace('\n', "\r\n"));
        self.log_text.push_str("\r\n");
        if self.log_text.len() > 100_000 {
            let mut cut = self.log_text.len() - 80_000;
            while !self.log_text.is_char_boundary(cut) {
                cut += 1;
            }
            self.log_text.drain(..cut);
        }
        set_text(self.log, &self.log_text);
        SendMessageW(self.log, EM_SETSEL, WPARAM(usize::MAX), LPARAM(-1));
        SendMessageW(self.log, EM_SCROLLCARET, WPARAM(0), LPARAM(0));
    }
    unsafe fn error(&mut self, error: impl std::fmt::Display) {
        self.append(&format!("错误：{error}"));
        MessageBoxW(
            self.hwnd,
            PCWSTR(wide(&error.to_string()).as_ptr()),
            w!("KMEmulator"),
            MB_OK | MB_ICONERROR,
        );
    }
    unsafe fn refresh(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.names = self.scripts.list()?;
        SendMessageW(self.list, LB_RESETCONTENT, WPARAM(0), LPARAM(0));
        for name in &self.names {
            SendMessageW(
                self.list,
                LB_ADDSTRING,
                WPARAM(0),
                LPARAM(wide(name).as_ptr() as isize),
            );
        }
        self.restore_selection();
        Ok(())
    }
    unsafe fn restore_selection(&self) {
        let index = self
            .names
            .iter()
            .position(|name| Some(name) == self.selected.as_ref())
            .map(|i| i as isize)
            .unwrap_or(-1);
        SendMessageW(self.list, LB_SETCURSEL, WPARAM(index as usize), LPARAM(0));
    }
    unsafe fn dirty(&self) -> bool {
        text(self.editor) != self.saved
    }
    unsafe fn save(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let filename = text(self.name);
        let path = self.scripts.path(&filename)?;
        let normalized = path.file_name().unwrap().to_string_lossy().to_string();
        let source = text(self.editor);
        if self.selected.as_deref() == Some(normalized.as_str()) {
            self.scripts.save(&normalized, &source)?;
        } else {
            self.scripts.create(&normalized, &source)?;
        }
        self.selected = Some(normalized.clone());
        self.saved = source;
        set_text(self.name, &normalized);
        self.refresh()?;
        self.append("脚本已保存");
        Ok(())
    }
    unsafe fn confirm_changes(&mut self) -> bool {
        if !self.dirty() {
            return true;
        }
        match MessageBoxW(
            self.hwnd,
            w!("当前脚本有未保存的修改，是否保存？"),
            w!("保存修改"),
            MB_YESNOCANCEL | MB_ICONQUESTION,
        ) {
            IDYES => match self.save() {
                Ok(()) => true,
                Err(error) => {
                    self.error(error);
                    false
                }
            },
            IDNO => true,
            _ => false,
        }
    }
    unsafe fn select(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let index = SendMessageW(self.list, LB_GETCURSEL, WPARAM(0), LPARAM(0)).0;
        let Some(name) = self.names.get(index as usize).cloned() else {
            return Ok(());
        };
        if self.selected.as_ref() == Some(&name) {
            return Ok(());
        }
        if !self.confirm_changes() {
            self.restore_selection();
            return Ok(());
        }
        let source = fs::read_to_string(self.scripts.path(&name)?)?;
        set_text(
            self.editor,
            &source.replace("\r\n", "\n").replace('\n', "\r\n"),
        );
        self.saved = text(self.editor);
        self.selected = Some(name.clone());
        set_text(self.name, &name);
        self.restore_selection();
        Ok(())
    }
    unsafe fn set_busy(&self, busy: bool) {
        for (id, hwnd) in &self.buttons {
            let enabled = if *id == STOP {
                busy
            } else if *id == CLEAR {
                true
            } else {
                !busy
            };
            EnableWindow(*hwnd, enabled);
        }
        EnableWindow(self.list, !busy);
        EnableWindow(self.name, !busy);
        SendMessageW(
            self.editor,
            EM_SETREADONLY,
            WPARAM(busy as usize),
            LPARAM(0),
        );
    }
    unsafe fn start(&mut self, mode: Mode) -> Result<(), Box<dyn std::error::Error>> {
        let source = text(self.editor);
        runtime::syntax(&source)?;
        self.stop = Arc::new(AtomicBool::new(false));
        let stop = self.stop.clone();
        let (tx, rx) = mpsc::sync_channel(1000);
        let name = text(self.name);
        self.append(match mode {
            Mode::Test => "开始模拟测试（不会发送系统输入）",
            Mode::Driver => "3 秒后运行到驱动，请切换到目标窗口；F8 随时停止",
        });
        self.worker = Some(std::thread::spawn(move || {
            if matches!(mode, Mode::Driver) {
                for _ in 0..300 {
                    if stop.load(Ordering::Relaxed) {
                        let _ = tx.send(Event::Finished(Err("已停止".into())));
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                runtime::execute(source, name, mode, stop, tx.clone())
            }))
            .unwrap_or_else(|_| Err("脚本执行线程发生异常，驱动连接已关闭".into()));
            let _ = tx.send(Event::Finished(result));
        }));
        self.receiver = Some(rx);
        self.set_busy(true);
        set_text(
            self.status,
            "执行中 · F8 / 停止按钮可终止 · 编辑内容为本次执行快照",
        );
        Ok(())
    }
    unsafe fn poll(&mut self) {
        let events: Vec<_> = self
            .receiver
            .as_ref()
            .map(|rx| rx.try_iter().take(200).collect())
            .unwrap_or_default();
        for event in events {
            match event {
                Event::Log(message) => self.append(&message),
                Event::Finished(result) => {
                    match result {
                        Ok(()) => {
                            self.append("执行完成");
                            set_text(self.status, "执行完成");
                        }
                        Err(error) => {
                            self.append(&format!("执行结束：{error}"));
                            set_text(self.status, "执行结束，详情见日志");
                        }
                    }
                    if let Some(worker) = self.worker.take() {
                        let _ = worker.join();
                    }
                    self.receiver = None;
                    self.set_busy(false);
                    if self.closing {
                        let _ = PostMessageW(self.hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
                    }
                }
            }
        }
    }
    unsafe fn command(
        &mut self,
        id: usize,
        notification: usize,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if id == STOP {
            self.stop.store(true, Ordering::Relaxed);
            set_text(self.status, "正在停止并释放按键…");
            return Ok(());
        }
        if id == CLEAR {
            self.log_text.clear();
            set_text(self.log, "");
            return Ok(());
        }
        if self.worker.is_some() {
            return Ok(());
        }
        match id {
            LIST if notification == LBN_SELCHANGE as usize => self.select()?,
            NEW => {
                if !self.confirm_changes() {
                    return Ok(());
                }
                self.selected = None;
                self.saved.clear();
                set_text(self.name, "新脚本.lua");
                set_text(self.editor, TEMPLATE);
                self.restore_selection();
            }
            SAVE => self.save()?,
            RENAME => {
                let old = self.selected.clone().ok_or("请先选择并保存脚本")?;
                let new = text(self.name);
                self.scripts.path(&new)?;
                if self.scripts.path(&new)?.exists() {
                    return Err("同名脚本已存在".into());
                }
                self.scripts.save(&old, &text(self.editor))?;
                self.saved = text(self.editor);
                self.scripts.rename(&old, &new)?;
                self.selected = Some(
                    self.scripts
                        .path(&new)?
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .to_string(),
                );
                set_text(self.name, self.selected.as_deref().unwrap());
                self.refresh()?;
            }
            DELETE => {
                let name = self.selected.clone().ok_or("请先选择脚本")?;
                if MessageBoxW(
                    self.hwnd,
                    PCWSTR(wide(&format!("删除脚本 {name}？未保存内容也会丢弃。")).as_ptr()),
                    w!("删除脚本"),
                    MB_YESNO | MB_ICONWARNING,
                ) == IDYES
                {
                    fs::remove_file(self.scripts.path(&name)?)?;
                    self.selected = None;
                    self.saved.clear();
                    set_text(self.editor, "");
                    set_text(self.name, "");
                    self.refresh()?;
                }
            }
            CHECK => {
                runtime::syntax(&text(self.editor))?;
                self.append("语法检查通过");
            }
            TEST => self.start(Mode::Test)?,
            RUN => self.start(Mode::Driver)?,
            CONNECT => {
                let _driver = Driver::open()?;
                self.append("KMDriver 已连接，协议版本 1");
                set_text(self.status, "驱动可用 · 协议版本 1");
            }
            _ => (),
        }
        Ok(())
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if message == WM_DESTROY {
        PostQuitMessage(0);
        return LRESULT(0);
    }
    if message == WM_GETMINMAXINFO {
        let bounds = &mut *(lp.0 as *mut MINMAXINFO);
        bounds.ptMinTrackSize = POINT { x: 940, y: 600 };
        return LRESULT(0);
    }
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    if !pointer.is_null() {
        // 模态对话框和控件通知会重入窗口过程，处理期间暂时解绑，避免重入取得第二个可变引用。
        struct DispatchGuard(HWND, isize);
        impl Drop for DispatchGuard {
            fn drop(&mut self) {
                unsafe {
                    SetWindowLongPtrW(self.0, GWLP_USERDATA, self.1);
                }
            }
        }
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        let _guard = DispatchGuard(hwnd, pointer as isize);
        let app = &mut *pointer;
        match message {
            WM_SIZE => {
                app.layout();
                return LRESULT(0);
            }
            WM_COMMAND => {
                if let Err(error) = app.command(wp.0 & 0xffff, wp.0 >> 16) {
                    app.error(error);
                }
                return LRESULT(0);
            }
            WM_TIMER => {
                app.poll();
                return LRESULT(0);
            }
            WM_HOTKEY => {
                app.stop.store(true, Ordering::Relaxed);
                return LRESULT(0);
            }
            WM_CLOSE => {
                if app.worker.is_some() {
                    app.closing = true;
                    app.stop.store(true, Ordering::Relaxed);
                    return LRESULT(0);
                }
                if !app.confirm_changes() {
                    return LRESULT(0);
                }
                let _ = DestroyWindow(hwnd);
                return LRESULT(0);
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                return LRESULT(0);
            }
            _ => (),
        }
    }
    DefWindowProcW(hwnd, message, wp, lp)
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let beside_exe = std::env::current_exe()?
        .parent()
        .ok_or("无法确定程序目录")?
        .join("scripts");
    let project_scripts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts");
    let directory = if let Some(path) = std::env::var_os("KMEMULATOR_SCRIPTS") {
        PathBuf::from(path)
    } else if beside_exe.exists() || !project_scripts.exists() {
        beside_exe
    } else {
        project_scripts
    };
    let scripts = Scripts::new(directory)?;
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: w!("KMEmulatorWindow"),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as isize),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(windows::core::Error::from_win32().into());
        }
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class.lpszClassName,
            w!("KMEmulator · Lua 脚本工作台"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1180,
            800,
            None,
            None,
            instance,
            None,
        );
        if hwnd.0 == 0 {
            return Err(windows::core::Error::from_win32().into());
        }
        let mut app = Box::new(App::new(hwnd, scripts));
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, (&mut *app as *mut App) as isize);
        app.refresh()?;
        app.layout();
        app.set_busy(false);
        app.append(&format!("脚本目录：{}", app.scripts.directory.display()));
        app.append("左侧管理脚本，上方编辑文件名，中间编辑 Lua，下方显示日志。\r\nAPI 示例：km.key_press('A') / km.sleep(200) / km.mouse_move(20,0) / km.mouse_click('left')");
        if !app.names.is_empty() {
            SendMessageW(app.list, LB_SETCURSEL, WPARAM(0), LPARAM(0));
            app.select()?;
        } else {
            set_text(app.name, "新脚本.lua");
            set_text(app.editor, TEMPLATE);
        }
        if RegisterHotKey(hwnd, 1, MOD_NOREPEAT, 0x77).is_err() {
            app.append("F8 全局停止快捷键被占用，请使用停止按钮");
        }
        SetTimer(hwnd, 1, 50, None);
        ShowWindow(hwnd, SW_SHOW);
        let mut message = MSG::default();
        loop {
            let result = GetMessageW(&mut message, None, 0, 0).0;
            if result <= 0 {
                break;
            }
            if !IsDialogMessageW(hwnd, &message).as_bool() {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        let _ = UnregisterHotKey(hwnd, 1);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
    }
    Ok(())
}
