#![windows_subsystem = "windows"]

use std::cell::RefCell;
use std::env;
use std::fs;
use std::iter::once;
use std::mem::{size_of, zeroed};
use std::path::PathBuf;
use std::ptr;

use serde::{Deserialize, Serialize};
use winapi::shared::minwindef::{BOOL, DWORD, HKEY, LPARAM, LRESULT, TRUE, UINT, WPARAM};
use winapi::shared::windef::{HICON, HWND, POINT, RECT};
use winapi::shared::winerror::ERROR_ALREADY_EXISTS;
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::handleapi::CloseHandle;
use winapi::um::libloaderapi::{GetModuleFileNameW, GetModuleHandleW};
use winapi::um::shellapi::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW,
};
use winapi::um::synchapi::CreateMutexW;
use winapi::um::winreg::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY_CURRENT_USER,
};
use winapi::um::winnt::{KEY_SET_VALUE, REG_SZ};
use winapi::um::winuser::*;

const WM_TRAY: UINT = WM_APP + 1;
const IDT_PIN: usize = 1;
const IDT_HELLO: usize = 2;
const ID_AUTO: usize = 1000;
const ID_WIN_BASE: usize = 1100;
const ID_CORNER_BASE: usize = 1200;
const ID_SIZE_BASE: usize = 1300;
const ID_TOPMOST: usize = 1401;
const ID_STARTUP: usize = 1402;
const ID_PINNOW: usize = 1403;
const ID_EXIT: usize = 1404;

const CORNER_NAMES: [&str; 4] = ["Sağ Alt", "Sağ Üst", "Sol Alt", "Sol Üst"];
const SIZE_NAMES: [&str; 3] = ["Mevcut Boyut", "Tam Yükseklik · 420 px", "Tam Yükseklik · 520 px"];

#[derive(Serialize, Deserialize, Clone)]
struct Settings {
    window_match: String,
    corner: usize,
    size_mode: usize,
    top_most: bool,
    run_at_startup: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            window_match: "Telegram".into(),
            corner: 0,
            size_mode: 0,
            top_most: true,
            run_at_startup: false,
        }
    }
}

struct State {
    settings: Settings,
    target: HWND,
    keep_w: i32,
    keep_h: i32,
    notified: bool,
    menu_windows: Vec<String>,
    settings_path: PathBuf,
}

thread_local! {
    static STATE: RefCell<Option<State>> = RefCell::new(None);
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(once(0)).collect()
}

fn set_wstr<const N: usize>(dst: &mut [u16; N], s: &str) {
    let w: Vec<u16> = s.encode_utf16().take(N - 1).collect();
    dst[..w.len()].copy_from_slice(&w);
    dst[w.len()] = 0;
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

fn checked(b: bool) -> UINT {
    if b {
        MF_CHECKED
    } else {
        0
    }
}

unsafe fn get_title(h: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = GetWindowTextW(h, buf.as_mut_ptr(), 256);
    if n <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..n as usize])
}

unsafe fn list_windows() -> Vec<(HWND, String)> {
    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
        let out = &mut *(l as *mut Vec<(HWND, String)>);
        if IsWindowVisible(h) != 0 {
            let ex = GetWindowLongW(h, GWL_EXSTYLE);
            if ex & (WS_EX_TOOLWINDOW as i32) == 0 {
                let t = get_title(h);
                if !t.trim().is_empty() {
                    out.push((h, t));
                }
            }
        }
        TRUE
    }
    let mut v: Vec<(HWND, String)> = Vec::new();
    EnumWindows(Some(cb), &mut v as *mut _ as LPARAM);
    v
}

unsafe fn find_target(m: &str) -> HWND {
    let wins = list_windows();
    let ml = m.to_lowercase();
    for (h, t) in &wins {
        if t.to_lowercase() == ml {
            return *h;
        }
    }
    for (h, t) in &wins {
        if t.to_lowercase().contains(&ml) {
            return *h;
        }
    }
    ptr::null_mut()
}

unsafe fn set_tip(hwnd: HWND, tip: &str) {
    let mut nid: NOTIFYICONDATAW = zeroed();
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as DWORD;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid.uFlags = NIF_TIP;
    set_wstr(&mut nid.szTip, tip);
    Shell_NotifyIconW(NIM_MODIFY, &mut nid);
}

unsafe fn balloon(hwnd: HWND, title: &str, msg: &str) {
    let mut nid: NOTIFYICONDATAW = zeroed();
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as DWORD;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid.uFlags = NIF_INFO;
    nid.dwInfoFlags = NIIF_INFO;
    set_wstr(&mut nid.szInfoTitle, title);
    set_wstr(&mut nid.szInfo, msg);
    Shell_NotifyIconW(NIM_MODIFY, &mut nid);
}

unsafe fn pin(hwnd: HWND) {
    STATE.with(|s| {
        let mut b = s.borrow_mut();
        let st = match b.as_mut() {
            Some(v) => v,
            None => return,
        };
        if st.target.is_null() || IsWindow(st.target) == 0 {
            st.target = find_target(&st.settings.window_match);
            st.keep_w = 0;
            st.keep_h = 0;
            if st.target.is_null() {
                let tip = format!("CornerPin · Bekleniyor: {}", truncate(&st.settings.window_match, 20));
                set_tip(hwnd, &tip);
                return;
            }
        }

        // Win + D / Win + M sonrası geri getir
        if IsIconic(st.target) != 0 {
            ShowWindow(st.target, SW_RESTORE);
        }
        if IsZoomed(st.target) != 0 {
            ShowWindow(st.target, SW_RESTORE);
        }

        let mut r: RECT = zeroed();
        GetWindowRect(st.target, &mut r);
        if st.keep_w == 0 {
            st.keep_w = (r.right - r.left).max(200);
            st.keep_h = (r.bottom - r.top).max(200);
        }

        let mut mi: MONITORINFO = zeroed();
        mi.cbSize = size_of::<MONITORINFO>() as DWORD;
        GetMonitorInfoW(MonitorFromWindow(st.target, MONITOR_DEFAULTTONEAREST), &mut mi);
        let wa = mi.rcWork;

        let (mut w, mut h) = (st.keep_w, st.keep_h);
        match st.settings.size_mode {
            1 => {
                w = 420;
                h = wa.bottom - wa.top;
            }
            2 => {
                w = 520;
                h = wa.bottom - wa.top;
            }
            _ => {}
        }

        let (x, y) = match st.settings.corner {
            1 => (wa.right - w, wa.top),
            2 => (wa.left, wa.bottom - h),
            3 => (wa.left, wa.top),
            _ => (wa.right - w, wa.bottom - h),
        };

        let ex = GetWindowLongW(st.target, GWL_EXSTYLE);
        let is_top = ex & (WS_EX_TOPMOST as i32) != 0;
        let want_top = st.settings.top_most;
        let pos_diff = (r.left - x).abs() > 1
            || (r.top - y).abs() > 1
            || ((r.right - r.left) - w).abs() > 1
            || ((r.bottom - r.top) - h).abs() > 1;

        if pos_diff || is_top != want_top {
            SetWindowPos(
                st.target,
                if want_top { HWND_TOPMOST } else { HWND_NOTOPMOST },
                x,
                y,
                w,
                h,
                SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_SHOWWINDOW,
            );
        }

        if !st.notified {
            st.notified = true;
            let msg = format!(
                "{} {} köşesine sabitlendi.",
                st.settings.window_match,
                CORNER_NAMES[st.settings.corner.min(3)]
            );
            balloon(hwnd, "CornerPin", &msg);
        }
        let title = get_title(st.target);
        set_tip(hwnd, &format!("CornerPin · {}", truncate(&title, 30)));
    });
}

unsafe fn show_menu(hwnd: HWND) {
    let menu = CreatePopupMenu();
    STATE.with(|s| {
        let mut b = s.borrow_mut();
        let st = match b.as_mut() {
            Some(v) => v,
            None => return,
        };

        let mwin = CreatePopupMenu();
        let auto_checked = st.settings.window_match.to_lowercase() == "telegram";
        AppendMenuW(
            mwin,
            MF_STRING | checked(auto_checked),
            ID_AUTO,
            wide("Telegram (Otomatik)").as_ptr(),
        );
        AppendMenuW(mwin, MF_SEPARATOR, 0, ptr::null());
        st.menu_windows = list_windows().into_iter().map(|(_, t)| t).take(40).collect();
        let cur = st.settings.window_match.to_lowercase();
        for (i, t) in st.menu_windows.iter().enumerate() {
            let short = if t.chars().count() > 50 {
                format!("{}…", t.chars().take(50).collect::<String>())
            } else {
                t.clone()
            };
            AppendMenuW(
                mwin,
                MF_STRING | checked(t.to_lowercase() == cur),
                ID_WIN_BASE + i,
                wide(&short).as_ptr(),
            );
        }
        AppendMenuW(menu, MF_POPUP, mwin as usize, wide("Pencere").as_ptr());

        let mcorner = CreatePopupMenu();
        for (i, name) in CORNER_NAMES.iter().enumerate() {
            AppendMenuW(
                mcorner,
                MF_STRING | checked(st.settings.corner == i),
                ID_CORNER_BASE + 1 + i,
                wide(name).as_ptr(),
            );
        }
        AppendMenuW(menu, MF_POPUP, mcorner as usize, wide("Köşe").as_ptr());

        let msize = CreatePopupMenu();
        for (i, name) in SIZE_NAMES.iter().enumerate() {
            AppendMenuW(
                msize,
                MF_STRING | checked(st.settings.size_mode == i),
                ID_SIZE_BASE + 1 + i,
                wide(name).as_ptr(),
            );
        }
        AppendMenuW(menu, MF_POPUP, msize as usize, wide("Boyut").as_ptr());

        AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
        AppendMenuW(
            menu,
            MF_STRING | checked(st.settings.top_most),
            ID_TOPMOST,
            wide("Her Zaman Üstte").as_ptr(),
        );
        AppendMenuW(
            menu,
            MF_STRING | checked(st.settings.run_at_startup),
            ID_STARTUP,
            wide("Windows İle Başlat").as_ptr(),
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
        AppendMenuW(menu, MF_STRING, ID_PINNOW, wide("Şimdi Sabitle").as_ptr());
        AppendMenuW(menu, MF_STRING, ID_EXIT, wide("Çıkış").as_ptr());
    });

    let mut pt: POINT = zeroed();
    GetCursorPos(&mut pt);
    SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
        pt.x,
        pt.y,
        0,
        hwnd,
        ptr::null(),
    );
    PostMessageW(hwnd, WM_NULL, 0, 0);
    DestroyMenu(menu);
    if cmd > 0 {
        handle_command(hwnd, cmd as usize);
    }
}

fn reset_target(st: &mut State) {
    st.target = ptr::null_mut();
    st.keep_w = 0;
    st.keep_h = 0;
    st.notified = false;
}

unsafe fn handle_command(hwnd: HWND, cmd: usize) {
    if cmd == ID_EXIT {
        DestroyWindow(hwnd);
        return;
    }
    STATE.with(|s| {
        let mut b = s.borrow_mut();
        let st = match b.as_mut() {
            Some(v) => v,
            None => return,
        };
        if cmd == ID_AUTO {
            st.settings.window_match = "Telegram".into();
            reset_target(st);
        } else if cmd > ID_CORNER_BASE && cmd <= ID_CORNER_BASE + 4 {
            st.settings.corner = cmd - ID_CORNER_BASE - 1;
        } else if cmd > ID_SIZE_BASE && cmd <= ID_SIZE_BASE + 3 {
            st.settings.size_mode = cmd - ID_SIZE_BASE - 1;
            st.keep_w = 0;
            st.keep_h = 0;
        } else if cmd == ID_TOPMOST {
            st.settings.top_most = !st.settings.top_most;
        } else if cmd == ID_STARTUP {
            st.settings.run_at_startup = !st.settings.run_at_startup;
        } else if cmd == ID_PINNOW {
            // sadece pin çağır
        } else if cmd >= ID_WIN_BASE && cmd < ID_WIN_BASE + 40 {
            let idx = cmd - ID_WIN_BASE;
            if idx < st.menu_windows.len() {
                st.settings.window_match = st.menu_windows[idx].clone();
                reset_target(st);
            }
        }
        save_settings(st);
        apply_startup(st.settings.run_at_startup);
    });
    pin(hwnd);
}

fn settings_path() -> PathBuf {
    let mut p = PathBuf::from(env::var("APPDATA").unwrap_or_else(|_| ".".into()));
    p.push("CornerPin");
    let _ = fs::create_dir_all(&p);
    p.push("settings.json");
    p
}

fn load_settings(p: &PathBuf) -> Settings {
    fs::read_to_string(p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_settings(st: &State) {
    if let Ok(json) = serde_json::to_string_pretty(&st.settings) {
        let _ = fs::write(&st.settings_path, json);
    }
}

unsafe fn apply_startup(enable: bool) {
    let mut key: HKEY = ptr::null_mut();
    let sub = wide("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run");
    if RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, KEY_SET_VALUE, &mut key) != 0 {
        return;
    }
    if enable {
        let mut buf = [0u16; 1024];
        let n = GetModuleFileNameW(ptr::null_mut(), buf.as_mut_ptr(), 1024);
        if n > 0 {
            let mut path: Vec<u16> = vec!['"' as u16];
            path.extend_from_slice(&buf[..n as usize]);
            path.push('"' as u16);
            path.push(0);
            let bytes: &[u8] =
                std::slice::from_raw_parts(path.as_ptr() as *const u8, path.len() * 2);
            RegSetValueExW(
                key,
                wide("CornerPin").as_ptr(),
                0,
                REG_SZ,
                bytes.as_ptr(),
                bytes.len() as DWORD,
            );
        }
    } else {
        RegDeleteValueW(key, wide("CornerPin").as_ptr());
    }
    RegCloseKey(key);
}

unsafe fn create_icon() -> HICON {
    let (w, h) = (32usize, 32usize);
    let mut xor = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            let (mut b, mut g, mut r) = (32u8, 26u8, 24u8);
            if x < 2 || y < 2 || x >= w - 2 || y >= h - 2 {
                b = 255;
                g = 160;
                r = 90;
            }
            if x >= 18 && y >= 18 && x < 31 && y < 31 {
                b = 60;
                g = 150;
                r = 255;
            }
            xor[i] = b;
            xor[i + 1] = g;
            xor[i + 2] = r;
            xor[i + 3] = 255;
        }
    }
    let and = vec![0u8; w * h / 8];
    let icon = CreateIcon(
        GetModuleHandleW(ptr::null()),
        w as i32,
        h as i32,
        1,
        32,
        and.as_ptr(),
        xor.as_ptr(),
    );
    if icon.is_null() {
        LoadIconW(ptr::null_mut(), IDI_APPLICATION)
    } else {
        icon
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: UINT, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            if w == IDT_HELLO {
                KillTimer(hwnd, IDT_HELLO);
                balloon(
                    hwnd,
                    "CornerPin",
                    "Çalışıyorum, beni tepside bulacaksın. Telegram açıksa birazdan köşeye sabitlenecek.",
                );
            } else if w == IDT_PIN {
                pin(hwnd);
            }
            0
        }
        WM_TRAY => {
            let ev = (l as u32) & 0xFFFF;
            if ev == WM_RBUTTONUP {
                show_menu(hwnd);
            } else if ev == WM_LBUTTONDBLCLK {
                pin(hwnd);
            }
            0
        }
        WM_DESTROY => {
            let mut nid: NOTIFYICONDATAW = zeroed();
            nid.cbSize = size_of::<NOTIFYICONDATAW>() as DWORD;
            nid.hWnd = hwnd;
            nid.uID = 1;
            Shell_NotifyIconW(NIM_DELETE, &mut nid);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}

fn main() {
    unsafe {
        let mutex = CreateMutexW(
            ptr::null_mut(),
            TRUE,
            wide("Local\\CornerPinSingleInstance").as_ptr(),
        );
        if !mutex.is_null() && GetLastError() == ERROR_ALREADY_EXISTS {
            MessageBoxW(
                ptr::null_mut(),
                wide("CornerPin zaten çalışıyor. Sağ altta saatin yanındaki tepsi simgesine bak; gizliyse ^ okuna tıkla.").as_ptr(),
                wide("CornerPin").as_ptr(),
                MB_OK | MB_ICONINFORMATION,
            );
            CloseHandle(mutex);
            return;
        }

        let hinst = GetModuleHandleW(ptr::null());
        let class = wide("CornerPinWnd");
        let wnd = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinst,
            lpszClassName: class.as_ptr(),
            ..zeroed()
        };
        RegisterClassW(&wnd);
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("CornerPin").as_ptr(),
            0,
            0,
            0,
            0,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            hinst,
            ptr::null_mut(),
        );
        if hwnd.is_null() {
            return;
        }

        let spath = settings_path();
        let settings = load_settings(&spath);
        let run_at_startup = settings.run_at_startup;
        let hicon = create_icon();

        let mut nid: NOTIFYICONDATAW = zeroed();
        nid.cbSize = size_of::<NOTIFYICONDATAW>() as DWORD;
        nid.hWnd = hwnd;
        nid.uID = 1;
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = hicon;
        set_wstr(&mut nid.szTip, "CornerPin");
        Shell_NotifyIconW(NIM_ADD, &mut nid);

        STATE.with(|s| {
            *s.borrow_mut() = Some(State {
                settings,
                target: ptr::null_mut(),
                keep_w: 0,
                keep_h: 0,
                notified: false,
                menu_windows: Vec::new(),
                settings_path: spath,
            });
        });
        apply_startup(run_at_startup);

        SetTimer(hwnd, IDT_PIN, 400, None);
        SetTimer(hwnd, IDT_HELLO, 700, None);

        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if !mutex.is_null() {
            CloseHandle(mutex);
        }
    }
}

