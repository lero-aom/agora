use std::{mem::zeroed, sync::OnceLock};

use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EnumWindows, GetClientRect, GetForegroundWindow, GetMessageW, GetWindow,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
    IsWindowVisible, SetForegroundWindow, TranslateMessage, GW_OWNER, MSG,
};

static FOREGROUND_EVENTS: OnceLock<tokio::sync::mpsc::UnboundedSender<()>> = OnceLock::new();

const EVENT_SYSTEM_FOREGROUND: u32 = 0x0003;
const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;
const WINEVENT_SKIPOWNPROCESS: u32 = 0x0002;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct GameWindow {
    pub(crate) pid: u32,
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) foreground: bool,
    pub(crate) minimized: bool,
}

struct EnumState {
    best: HWND,
    best_area: i64,
}

pub(crate) fn become_dpi_aware() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

pub(crate) fn detect_aom_window() -> Option<GameWindow> {
    let mut state = EnumState {
        best: std::ptr::null_mut(),
        best_area: -1,
    };
    unsafe {
        EnumWindows(Some(enum_window), &mut state as *mut EnumState as LPARAM);
    }
    if state.best.is_null() {
        None
    } else {
        game_window_rect(state.best)
    }
}

pub(crate) fn focus_aom_window() -> bool {
    let Some(hwnd) = detect_aom_hwnd() else {
        return false;
    };
    unsafe { SetForegroundWindow(hwnd) != 0 }
}

pub(crate) fn start_foreground_event_watcher(sender: tokio::sync::mpsc::UnboundedSender<()>) {
    if FOREGROUND_EVENTS.set(sender).is_err() {
        notify_foreground_changed();
        return;
    }

    notify_foreground_changed();
    std::thread::spawn(move || unsafe {
        let hook = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            std::ptr::null_mut(),
            Some(foreground_event_hook),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        if hook.is_null() {
            return;
        }

        let mut message: MSG = zeroed();
        while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        UnhookWinEvent(hook);
    });
}

unsafe extern "system" fn foreground_event_hook(
    _hook: HWINEVENTHOOK,
    event: u32,
    _hwnd: HWND,
    _object_id: i32,
    _child_id: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if event == EVENT_SYSTEM_FOREGROUND {
        notify_foreground_changed();
    }
}

fn notify_foreground_changed() {
    if let Some(sender) = FOREGROUND_EVENTS.get() {
        let _ = sender.send(());
    }
}

fn detect_aom_hwnd() -> Option<HWND> {
    let mut state = EnumState {
        best: std::ptr::null_mut(),
        best_area: -1,
    };
    unsafe {
        EnumWindows(Some(enum_window), &mut state as *mut EnumState as LPARAM);
    }
    if state.best.is_null() {
        None
    } else {
        Some(state.best)
    }
}

fn game_window_rect(hwnd: HWND) -> Option<GameWindow> {
    unsafe {
        if !is_candidate_aom_window(hwnd) {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if IsIconic(hwnd) != 0 {
            return Some(GameWindow {
                pid,
                minimized: true,
                ..GameWindow::default()
            });
        }
        let mut rect: RECT = zeroed();
        if GetClientRect(hwnd, &mut rect) == 0 {
            return None;
        }
        let mut origin = POINT { x: 0, y: 0 };
        if ClientToScreen(hwnd, &mut origin) == 0 {
            return None;
        }
        Some(GameWindow {
            pid,
            x: origin.x,
            y: origin.y,
            width: rect.right - rect.left,
            height: rect.bottom - rect.top,
            foreground: foreground_process_id() == Some(pid),
            minimized: false,
        })
    }
}

fn foreground_process_id() -> Option<u32> {
    unsafe {
        let foreground = GetForegroundWindow();
        if foreground.is_null() {
            return None;
        }

        let mut pid = 0u32;
        GetWindowThreadProcessId(foreground, &mut pid);
        (pid != 0).then_some(pid)
    }
}

unsafe extern "system" fn enum_window(hwnd: HWND, lparam: LPARAM) -> i32 {
    if !is_candidate_aom_window(hwnd) {
        return 1;
    }

    let state = &mut *(lparam as *mut EnumState);
    if GetForegroundWindow() == hwnd {
        state.best = hwnd;
        return 1;
    }

    let mut rect: RECT = zeroed();
    if GetClientRect(hwnd, &mut rect) == 0 {
        return 1;
    }
    let area = i64::from(rect.right - rect.left) * i64::from(rect.bottom - rect.top);
    if area > state.best_area {
        state.best_area = area;
        state.best = hwnd;
    }
    1
}

unsafe fn is_candidate_aom_window(hwnd: HWND) -> bool {
    if IsWindow(hwnd) == 0 || IsWindowVisible(hwnd) == 0 || !GetWindow(hwnd, GW_OWNER).is_null() {
        return false;
    }

    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid == 0 {
        return false;
    }

    match process_image_path(pid) {
        Some(path) => is_aom_process_image_path(&path),
        None => window_title(hwnd).is_some_and(|title| is_aom_window_title(&title)),
    }
}

fn process_image_path(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }

        let mut buffer = vec![0u16; 32_768];
        let mut len = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut len);
        CloseHandle(handle);

        if ok == 0 || len == 0 {
            return None;
        }

        Some(String::from_utf16_lossy(&buffer[..len as usize]))
    }
}

fn is_aom_process_image_path(path: &str) -> bool {
    let file_name = path
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase();
    matches!(file_name.as_str(), "aomrt_s.exe" | "aomrt_s")
}

unsafe fn window_title(hwnd: HWND) -> Option<String> {
    let len = GetWindowTextLengthW(hwnd);
    if len <= 0 {
        return None;
    }
    let mut buffer = vec![0u16; len as usize + 1];
    let copied = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
    if copied <= 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buffer[..copied as usize]))
}

fn is_aom_window_title(title: &str) -> bool {
    let title = title.trim().to_ascii_lowercase();
    title.contains("age of mythology") && title.contains("retold")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_aom_retold_window_titles() {
        assert!(is_aom_window_title("Age of Mythology: Retold"));
        assert!(is_aom_window_title("AGE OF MYTHOLOGY RETOLD"));
        assert!(!is_aom_window_title(
            "Age of Empires II: Definitive Edition"
        ));
    }

    #[test]
    fn recognizes_aom_process_image_paths() {
        assert!(is_aom_process_image_path(
            r#"Z:\SteamLibrary\steamapps\common\Age of Mythology Retold\AoMRT_s.exe"#
        ));
        assert!(!is_aom_process_image_path(
            r#"C:\Program Files\Mozilla Firefox\firefox.exe"#
        ));
    }
}
