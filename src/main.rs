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
use winapi::shared::windef::{HICON, HMONITOR, HWND, POINT, RECT};
use winapi::shared::winerror::ERROR_ALREADY_EXISTS;
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::handleapi::CloseHandle;
use winapi::um::libloaderapi::{GetModuleFileNameW, GetModuleHandleW};
use winapi::um::shellapi::{
    Shell_NotifyIconW, SHAppBarMessage, ABE_LEFT, ABE_RIGHT, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE,
    ABM_SETPOS, ABN_POSCHANGED, APPBARDATA, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO,
    NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
};
use winapi::um::synchapi::CreateMutexW;
use winapi::um::winreg::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY_CURRENT_USER,
};
use winapi::um::winnt::{KEY_SET_VALUE, REG_SZ};
use winapi::um::winuser::*;

const WM_TRAY: UINT = WM_APP + 1;
const WM_BAR: UINT = WM_APP + 2;
const IDT_PIN: usize = 1;
const IDT_HELLO: usize = 2;
const ID_AUTO: usize = 1000;
const ID_WIN_BASE: usize = 1100;
const ID_EDGE_BASE: usize = 1200;
const ID_WIDTH_BASE: usize = 1300;
const ID_TOPMOST: usize = 1401;
const ID_STARTUP: usize = 1402;
const ID_PINNOW: usize = 1403;
const ID_EXIT: usize = 1404;

const EDGE_NAMES: [&str; 2] = ["Sağ", "Sol"];
const WIDTH_NAMES: [&str; 3] = ["Mevcut Genişlik", "420 px", "520 px"];

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct Settings {
    window_match: String,
    edge: usize,       // 0 = Sağ, 1 = Sol
    width_mode: usize, // 0 = mevcut, 1 = 420, 2 = 520
    top_most: bool,
    run_at_startup: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            window_match: "Telegram".into(),
            edge: 0,
            width_mode: 0,
            top_most: true,
            run_at_startup: false,
        }
    }
}

struct State {
    settings: Settings,
    target: HWND,
    bar: HWND,
    bar_registered: bool,
    bar_rect: RECT,
    bar_dirty: bool,
    keep_w: i32,
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

fn rects_equal(a: &RECT, b: &RECT) -> bool {
    a.left == b.left && a.top == b.top && a.right == b.right && a.bottom == b.bottom
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

// Ekranın kenarında kalıcı alan rezerve et (görev çubuğu gibi).
// Diğer pencereler maximize/snap edildiğinde bu alana giremez.
unsafe fn appbar_ensure(st: &mut State, mon: HMONITOR, width: i32) -> RECT {
    let mut mi: MONITORINFO = zeroed();
    mi.cbSize = size_of::<MONITORINFO>() as DWORD;
    GetMonitorInfoW(mon, &mut mi);
    let mut rc = mi.rcMonitor;
    let right_edge = st.settings.edge == 0;
    if right_edge {
        rc.left = rc.right - width;
    } else {
        rc.right = rc.left + width;
    }

    if !st.bar_registered {
        let mut abd: APPBARDATA = zeroed();
        abd.cbSize = size_of::<APPBARDATA>() as DWORD;
        abd.hWnd = st.bar;
        abd.uCallbackMessage = WM_BAR;
        SHAppBarMessage(ABM_NEW, &mut abd);
        st.bar_registered = true;
        st.bar_dirty = true;
    }

    if !st.bar_dirty && rects_equal(&st.bar_rect, &rc) {
        return st.bar_rect;
    }

    let mut abd: APPBARDATA = zeroed();
    abd.cbSize = size_of::<APPBARDATA>() as DWORD;
    abd.hWnd = st.bar;
    abd.uCallbackMessage = WM_BAR;
    abd.uEdge = if right_edge { ABE_RIGHT } else { ABE_LEFT };
    abd.rc = rc;
    SHAppBarMessage(ABM_QUERYPOS, &mut abd);
    // QUERYPOS kenarı görev çubuğuna göre kısaltır, genişliği biz yeniden uygularız
    if right_edge {
        abd.rc.left = abd.rc.right - width;
    } else {
        abd.rc.right = abd.rc.left + width;
    }
    SHAppBarMessage(ABM_SETPOS, &mut abd);
    let rc = abd.rc;
    SetWindowPos(
        st.bar,
        HWND_TOPMOST,
        rc.left,
        rc.top,
        rc.right - rc.left,
        rc.bottom - rc.top,
        SWP_NOACTIVATE | SWP_SHOWWINDOW,
    );
    st.bar_rect = rc;
    st.bar_dirty = false;
    rc
}

unsafe fn appbar_release(st: &mut State) {
    if st.bar_registered {
        let mut abd: APPBARDATA = zeroed();
        abd.cbSize = size_of::<APPBARDATA>() as DWORD;
        abd.hWnd = st.bar;
        SHAppBarMessage(ABM_REMOVE, &mut abd);
        st.bar_registered = false;
    }
    if !st.bar.is_null() {
        ShowWindow(st.bar, SW_HIDE);
    }
    st.bar_rect = zeroed();
    st.bar_dirty = true;
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
            if st.target.is_null() {
                appbar_release(st);
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

        let width = match st.settings.width_mode {
            1 => 420,
            2 => 520,
            _ => {
                if st.keep_w == 0 {
                    let mut r: RECT = zeroed();
                    GetWindowRect(st.target, &mut r);
                    st.keep_w = (r.right - r.left).max(280);
                }
                st.keep_w
            }
        };

        let mon = MonitorFromWindow(st.target, MONITOR_DEFAULTTONEAREST);
        let rc = appbar_ensure(st, mon, width);
        let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);

        let mut r: RECT = zeroed();
        GetWindowRect(st.target, &mut r);
        let ex = GetWindowLongW(st.target, GWL_EXSTYLE);
        let is_top = ex & (WS_EX_TOPMOST as i32) != 0;
        let want_top = st.settings.top_most;
        let pos_diff = (r.left - rc.left).abs() > 1
            || (r.top - rc.top).abs() > 1
            || ((r.right - r.left) - w).abs() > 1
            || ((r.bottom - r.top) - h).abs() > 1;

        if pos_diff || is_top != want_top {
            SetWindowPos(
                st.target,
                if want_top { HWND_TOPMOST } else { HWND_NOTOPMOST },
                rc.left,
                rc.top,
                w,
                h,
                SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_SHOWWINDOW,
            );
        }

        if !st.notified {
            st.notified = true;
            let msg = format!(
                "{} {} kenarına sabitlendi. Diğer pencereler artık onun alanına giremez.",
                st.settings.window_match,
                EDGE_NAMES[st.settings.edge.min(1)]
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

        let medge = CreatePopupMenu();
        for (i, name) in EDGE_NAMES.iter().enumerate() {
            AppendMenuW(
                medge,
                MF_STRING | checked(st.settings.edge == i),
                ID_EDGE_BASE + 1 + i,
                wide(name).as_ptr(),
            );
        }
        AppendMenuW(menu, MF_POPUP, medge as usize, wide("Kenar").as_ptr());

        let mwidth = CreatePopupMenu();
        for (i, name) in WIDTH_NAMES.iter().enumerate() {
            AppendMenuW(
                mwidth,
                MF_STRING | checked(st.settings.width_mode == i),
                ID_WIDTH_BASE + 1 + i,
                wide(name).as_ptr(),
            );
        }
        AppendMenuW(menu, MF_POPUP, mwidth as usize, wide("Genişlik").as_ptr());

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
        } else if cmd > ID_EDGE_BASE && cmd <= ID_EDGE_BASE + 2 {
            st.settings.edge = cmd - ID_EDGE_BASE - 1;
            st.bar_dirty = true;
        } else if cmd > ID_WIDTH_BASE && cmd <= ID_WIDTH_BASE + 3 {
            st.settings.width_mode = cmd - ID_WIDTH_BASE - 1;
            st.keep_w = 0;
            st.bar_dirty = true;
        } else if cmd == ID_TOPMOST {
            st.settings.top_most = !st.settings.top_most;
        } else if cmd == ID_STARTUP {
            st.settings.run_at_startup = !st.settings.run_at_startup;
        } else if cmd == ID_PINNOW {
            st.bar_dirty = true;
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

// Rezerv penceresi: görünmez (arka plansız), tıklamaları geçirir.
// Sadece ekran kenarında alan ayırmak için var, üstünde Telegram durur.
unsafe extern "system" fn bar_proc(hwnd: HWND, msg: UINT, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_NCHITTEST => HTTRANSPARENT as LRESULT,
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            BeginPaint(hwnd, &mut ps);
            EndPaint(hwnd, &ps);
            0
        }
        WM_BAR => {
            if w == ABN_POSCHANGED as WPARAM {
                STATE.with(|s| {
                    if let Some(st) = s.borrow_mut().as_mut() {
                        st.bar_dirty = true;
                    }
                });
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
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
                    "Çalışıyorum, beni tepside bulacaksın. Telegram açıksa birazdan kenara sabitlenecek.",
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
            STATE.with(|s| {
                if let Some(st) = s.borrow_mut().as_mut() {
                    appbar_release(st);
                }
            });
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

        // Ana gizli pencere (tepsi + zamanlayıcı sahibi)
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

        // Rezerv (appbar) penceresi
        let barclass = wide("CornerPinBar");
        let bwnd = WNDCLASSW {
            lpfnWndProc: Some(bar_proc),
            hInstance: hinst,
            lpszClassName: barclass.as_ptr(),
            ..zeroed()
        };
        RegisterClassW(&bwnd);
        let bar = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            barclass.as_ptr(),
            wide("CornerPinBar").as_ptr(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            hinst,
            ptr::null_mut(),
        );

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
                bar,
                bar_registered: false,
                bar_rect: zeroed(),
                bar_dirty: true,
                keep_w: 0,
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


