use super::{game, AppTab};
use dioxus::desktop::tao::platform::windows::WindowExtWindows;
use dioxus::prelude::{Readable, Signal, Writable};
use global_hotkey::hotkey::{Code as HotKeyCode, HotKey, Modifiers as HotKeyModifiers};
use tokio::sync::mpsc::UnboundedReceiver;
use windows_sys::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_RETURN};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowLongPtrW, IsChild, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_EX_APPWINDOW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
};

const GAME_WATCH_INTERVAL_MS: u64 = 75;
const OVERLAY_PASSIVE_WIDTH: i32 = 420;
const OVERLAY_PASSIVE_HEIGHT: i32 = 88;
const OVERLAY_INTERACTIVE_WIDTH: i32 = 900;
const OVERLAY_INTERACTIVE_HEIGHT: i32 = 560;
const OVERLAY_MARGIN: i32 = 24;
const OVERLAY_PASSIVE_TOP_OFFSET: i32 = 22;
const TASKBAR_ANCHOR_POSITION: i32 = -32_000;
const TASKBAR_ANCHOR_SIZE: i32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ShellMode {
    Tray,
    OverlayPassive,
    OverlayInteractive,
}

pub(super) fn initial_shell_mode(standalone_local_dev: bool) -> ShellMode {
    if standalone_local_dev {
        ShellMode::OverlayInteractive
    } else {
        ShellMode::Tray
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OverlayWindowCommand {
    None,
    ApplyPassive(game::GameWindow),
    ApplyInteractive(game::GameWindow),
    HideToTray,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GameWatchStep {
    detected: Option<game::GameWindow>,
    shell_mode: ShellMode,
    overlay_interactive: bool,
    command: OverlayWindowCommand,
}

pub(super) async fn watch_game_window(
    desktop: dioxus::desktop::DesktopContext,
    mut game_window: Signal<Option<game::GameWindow>>,
    mut shell_mode: Signal<ShellMode>,
    mut overlay_interactive: Signal<bool>,
    mut foreground_events: UnboundedReceiver<()>,
) {
    loop {
        let raw_detected = game::detect_aom_window();
        let previous = *game_window.read();
        let desktop_foreground = desktop_is_foreground(&desktop);
        let step = game_watch_step(
            *shell_mode.read(),
            *overlay_interactive.read(),
            previous,
            raw_detected,
            desktop_foreground,
        );

        game_window.set(step.detected);
        if *overlay_interactive.read() != step.overlay_interactive {
            overlay_interactive.set(step.overlay_interactive);
        }
        if *shell_mode.read() != step.shell_mode {
            shell_mode.set(step.shell_mode);
        }
        apply_overlay_window_command(&desktop, step.command);

        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_millis(GAME_WATCH_INTERVAL_MS)) => {}
            event = foreground_events.recv() => {
                if event.is_none() {
                    tokio::time::sleep(std::time::Duration::from_millis(GAME_WATCH_INTERVAL_MS)).await;
                }
            }
        }
    }
}

fn game_watch_step(
    shell_mode: ShellMode,
    overlay_interactive: bool,
    previous: Option<game::GameWindow>,
    raw_detected: Option<game::GameWindow>,
    desktop_foreground: bool,
) -> GameWatchStep {
    let detected = visible_detected_game_window(raw_detected);
    match detected {
        Some(bounds) if overlay_interactive && (desktop_foreground || bounds.foreground) => {
            GameWatchStep {
                detected,
                shell_mode: ShellMode::OverlayInteractive,
                overlay_interactive: true,
                command: if shell_mode != ShellMode::OverlayInteractive
                    || overlay_bounds_changed(previous, bounds)
                {
                    OverlayWindowCommand::ApplyInteractive(bounds)
                } else {
                    OverlayWindowCommand::None
                },
            }
        }
        Some(_) if overlay_interactive => GameWatchStep {
            detected,
            shell_mode: ShellMode::Tray,
            overlay_interactive: true,
            command: hide_to_tray_command(shell_mode),
        },
        Some(bounds) if bounds.foreground => GameWatchStep {
            detected,
            shell_mode: ShellMode::OverlayPassive,
            overlay_interactive: false,
            command: if shell_mode != ShellMode::OverlayPassive
                || overlay_bounds_changed(previous, bounds)
            {
                OverlayWindowCommand::ApplyPassive(bounds)
            } else {
                OverlayWindowCommand::None
            },
        },
        Some(_) => GameWatchStep {
            detected,
            shell_mode: ShellMode::Tray,
            overlay_interactive: false,
            command: hide_to_tray_command(shell_mode),
        },
        None => GameWatchStep {
            detected: None,
            shell_mode: ShellMode::Tray,
            overlay_interactive: false,
            command: hide_to_tray_command(shell_mode),
        },
    }
}

fn hide_to_tray_command(shell_mode: ShellMode) -> OverlayWindowCommand {
    if shell_mode == ShellMode::Tray {
        OverlayWindowCommand::None
    } else {
        OverlayWindowCommand::HideToTray
    }
}

fn visible_detected_game_window(
    raw_detected: Option<game::GameWindow>,
) -> Option<game::GameWindow> {
    raw_detected.filter(|window| !window.minimized && window.width > 0 && window.height > 0)
}

fn focused_visible_game_window(raw_detected: Option<game::GameWindow>) -> Option<game::GameWindow> {
    visible_detected_game_window(raw_detected).filter(|window| window.foreground)
}

fn apply_overlay_window_command(
    desktop: &dioxus::desktop::DesktopContext,
    command: OverlayWindowCommand,
) {
    match command {
        OverlayWindowCommand::None => {}
        OverlayWindowCommand::ApplyPassive(bounds) => apply_passive_overlay_window(desktop, bounds),
        OverlayWindowCommand::ApplyInteractive(bounds) => {
            apply_interactive_overlay_window(desktop, bounds);
        }
        OverlayWindowCommand::HideToTray => hide_to_tray(desktop),
    }
}

pub(super) async fn watch_tray_events(desktop: dioxus::desktop::DesktopContext) {
    loop {
        while dioxus::desktop::trayicon::TrayIconEvent::receiver()
            .try_recv()
            .is_ok()
        {
            // Tray clicks are intentionally ignored; Agora is overlay-only.
            hide_to_tray(&desktop);
        }

        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    }
}

pub(super) fn register_global_shortcuts(
    desktop: dioxus::desktop::DesktopContext,
    active_tab: Signal<AppTab>,
    shell_mode: Signal<ShellMode>,
    game_window: Signal<Option<game::GameWindow>>,
    overlay_interactive: Signal<bool>,
    mut ctrl_enter_down: Signal<bool>,
    standalone_local_dev: bool,
) -> Option<dioxus::desktop::ShortcutHandle> {
    if standalone_local_dev {
        return None;
    }
    let shortcut_desktop = desktop.clone();
    desktop
        .create_shortcut(
            HotKey::new(Some(HotKeyModifiers::CONTROL), HotKeyCode::Enter),
            move || {
                let keys_down = ctrl_enter_keys_down();
                if !keys_down {
                    ctrl_enter_down.set(false);
                    return;
                }
                if *ctrl_enter_down.read() {
                    return;
                }
                ctrl_enter_down.set(true);

                if *overlay_interactive.read() {
                    let aom_foreground =
                        visible_aom_window().is_some_and(|bounds| bounds.foreground);
                    if desktop_is_foreground(&shortcut_desktop) || aom_foreground {
                        close_interaction_or_hide(
                            &shortcut_desktop,
                            shell_mode,
                            game_window,
                            overlay_interactive,
                        );
                    }
                } else {
                    open_typing_mode_if_game_focused(
                        &shortcut_desktop,
                        active_tab,
                        shell_mode,
                        game_window,
                        overlay_interactive,
                    );
                }
            },
        )
        .ok()
}

fn ctrl_enter_keys_down() -> bool {
    unsafe { GetAsyncKeyState(VK_CONTROL as i32) < 0 && GetAsyncKeyState(VK_RETURN as i32) < 0 }
}

fn desktop_is_foreground(desktop: &dioxus::desktop::DesktopContext) -> bool {
    let hwnd = desktop.window.hwnd() as windows_sys::Win32::Foundation::HWND;
    unsafe {
        let foreground = GetForegroundWindow();
        !foreground.is_null() && (foreground == hwnd || IsChild(hwnd, foreground) != 0)
    }
}

fn open_typing_mode_if_game_focused(
    desktop: &dioxus::desktop::DesktopContext,
    mut active_tab: Signal<AppTab>,
    mut shell_mode: Signal<ShellMode>,
    mut game_window: Signal<Option<game::GameWindow>>,
    mut overlay_interactive: Signal<bool>,
) {
    let raw_detected = game::detect_aom_window();
    let detected = visible_detected_game_window(raw_detected);
    game_window.set(detected);

    if let Some(bounds) = focused_visible_game_window(raw_detected) {
        overlay_interactive.set(true);
        active_tab.set(AppTab::Global);
        shell_mode.set(ShellMode::OverlayInteractive);
        apply_interactive_overlay_window(desktop, bounds);
    }
}

fn visible_aom_window() -> Option<game::GameWindow> {
    visible_detected_game_window(game::detect_aom_window())
}

fn focus_or_get_foreground_aom_window(
    detected: Option<game::GameWindow>,
) -> Option<game::GameWindow> {
    let bounds = detected?;
    if bounds.foreground {
        return Some(bounds);
    }

    if !game::focus_aom_window() {
        return None;
    }

    visible_aom_window().filter(|bounds| bounds.foreground)
}

pub(super) fn close_interaction_or_hide(
    desktop: &dioxus::desktop::DesktopContext,
    mut shell_mode: Signal<ShellMode>,
    mut game_window: Signal<Option<game::GameWindow>>,
    mut overlay_interactive: Signal<bool>,
) {
    overlay_interactive.set(false);
    let detected = visible_aom_window();
    game_window.set(detected);
    if let Some(bounds) = focus_or_get_foreground_aom_window(detected) {
        shell_mode.set(ShellMode::OverlayPassive);
        apply_passive_overlay_window(desktop, bounds);
    } else {
        shell_mode.set(ShellMode::Tray);
        hide_to_tray(desktop);
    }
}

fn overlay_bounds_changed(previous: Option<game::GameWindow>, current: game::GameWindow) -> bool {
    !matches!(
        previous,
        Some(previous)
            if previous.pid == current.pid
                && previous.x == current.x
                && previous.y == current.y
                && previous.width == current.width
                && previous.height == current.height
                && previous.minimized == current.minimized
    )
}

fn apply_passive_overlay_window(
    desktop: &dioxus::desktop::DesktopContext,
    bounds: game::GameWindow,
) {
    desktop.window.set_enable(true);
    let _ = desktop.set_ignore_cursor_events(false);
    desktop.set_decorations(false);
    desktop.set_resizable(true);
    desktop.set_always_on_top(true);
    set_overlay_background(desktop);
    set_window_rect(
        desktop,
        bounds,
        OVERLAY_PASSIVE_WIDTH,
        OVERLAY_PASSIVE_HEIGHT,
    );
    desktop.set_resizable(false);
    apply_passive_native_window_style(desktop);
    set_dwm_border_color(desktop, DWMWA_COLOR_NONE);
    let _ = desktop.set_ignore_cursor_events(true);
    desktop.set_visible(true);
}

fn apply_interactive_overlay_window(
    desktop: &dioxus::desktop::DesktopContext,
    bounds: game::GameWindow,
) {
    apply_typing_native_window_style(desktop);
    set_dwm_border_color(desktop, DWMWA_COLOR_NONE);
    let _ = desktop.set_ignore_cursor_events(false);
    desktop.set_decorations(false);
    desktop.set_resizable(false);
    desktop.set_always_on_top(true);
    set_overlay_background(desktop);
    set_window_rect(
        desktop,
        bounds,
        OVERLAY_INTERACTIVE_WIDTH,
        OVERLAY_INTERACTIVE_HEIGHT,
    );
    desktop.set_visible(true);
    if !desktop_is_foreground(desktop) {
        desktop.set_focus();
    }
    set_dwm_border_color(desktop, DWMWA_COLOR_NONE);
}

pub(super) fn hide_to_tray(desktop: &dioxus::desktop::DesktopContext) {
    desktop.set_visible(false);
    let _ = desktop.set_ignore_cursor_events(true);
    desktop.set_decorations(false);
    desktop.set_resizable(false);
    desktop.set_always_on_top(false);
    set_taskbar_anchor_background(desktop);
    set_taskbar_anchor_rect(desktop);
    apply_tray_native_window_style(desktop);
    desktop.set_visible(true);
}

fn set_overlay_background(desktop: &dioxus::desktop::DesktopContext) {
    desktop.window.set_background_color(Some((7, 9, 9, 255)));
}

fn set_taskbar_anchor_background(desktop: &dioxus::desktop::DesktopContext) {
    desktop.window.set_background_color(Some((7, 9, 9, 0)));
}

fn apply_passive_native_window_style(desktop: &dioxus::desktop::DesktopContext) {
    desktop.window.set_enable(false);
    set_overlay_ex_style(
        desktop,
        WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_APPWINDOW,
        WS_EX_TOOLWINDOW,
    );
    let _ = desktop.window.set_skip_taskbar(false);
}

fn apply_typing_native_window_style(desktop: &dioxus::desktop::DesktopContext) {
    desktop.window.set_enable(true);
    set_overlay_ex_style(
        desktop,
        WS_EX_APPWINDOW,
        WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW,
    );
    let _ = desktop.window.set_skip_taskbar(false);
}

fn apply_tray_native_window_style(desktop: &dioxus::desktop::DesktopContext) {
    desktop.window.set_enable(true);
    set_overlay_ex_style(
        desktop,
        WS_EX_NOACTIVATE | WS_EX_APPWINDOW,
        WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
    );
    let _ = desktop.window.set_skip_taskbar(false);
}

fn set_overlay_ex_style(desktop: &dioxus::desktop::DesktopContext, add: u32, remove: u32) {
    let hwnd = desktop.window.hwnd() as windows_sys::Win32::Foundation::HWND;
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let updated = (style | add) & !remove;
        if updated != style {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, updated as isize);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
            );
        }
    }
}

fn set_dwm_border_color(desktop: &dioxus::desktop::DesktopContext, color: u32) {
    let hwnd = desktop.window.hwnd() as windows_sys::Win32::Foundation::HWND;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR as u32,
            &color as *const u32 as *const std::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
        );
    }
}

fn set_window_rect(
    desktop: &dioxus::desktop::DesktopContext,
    bounds: game::GameWindow,
    width: i32,
    height: i32,
) {
    let (x, y) = overlay_position(bounds, width, height);
    desktop.set_inner_size(dioxus::desktop::tao::dpi::PhysicalSize::new(
        width.max(1) as u32,
        height.max(1) as u32,
    ));
    desktop.set_outer_position(dioxus::desktop::tao::dpi::PhysicalPosition::new(x, y));
    let hwnd = desktop.window.hwnd() as windows_sys::Win32::Foundation::HWND;
    unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            x,
            y,
            width.max(1),
            height.max(1),
            SWP_NOACTIVATE | SWP_NOZORDER,
        );
    }
}

fn set_taskbar_anchor_rect(desktop: &dioxus::desktop::DesktopContext) {
    desktop.set_inner_size(dioxus::desktop::tao::dpi::PhysicalSize::new(
        TASKBAR_ANCHOR_SIZE as u32,
        TASKBAR_ANCHOR_SIZE as u32,
    ));
    desktop.set_outer_position(dioxus::desktop::tao::dpi::PhysicalPosition::new(
        TASKBAR_ANCHOR_POSITION,
        TASKBAR_ANCHOR_POSITION,
    ));

    let hwnd = desktop.window.hwnd() as windows_sys::Win32::Foundation::HWND;
    unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            TASKBAR_ANCHOR_POSITION,
            TASKBAR_ANCHOR_POSITION,
            TASKBAR_ANCHOR_SIZE,
            TASKBAR_ANCHOR_SIZE,
            SWP_NOACTIVATE | SWP_NOZORDER,
        );
    }
}

fn overlay_position(bounds: game::GameWindow, width: i32, height: i32) -> (i32, i32) {
    let centered_x = bounds.x + ((bounds.width - width) / 2);
    let min_x = bounds.x + OVERLAY_MARGIN;
    let max_x = bounds.x + bounds.width - width - OVERLAY_MARGIN;
    let target_x = if width == OVERLAY_PASSIVE_WIDTH && height == OVERLAY_PASSIVE_HEIGHT {
        centered_x + ((max_x - centered_x) / 2)
    } else {
        centered_x
    };
    let x = if min_x <= max_x {
        target_x.clamp(min_x, max_x)
    } else {
        target_x
    };
    let y = if width == OVERLAY_PASSIVE_WIDTH && height == OVERLAY_PASSIVE_HEIGHT {
        bounds.y + OVERLAY_PASSIVE_TOP_OFFSET
    } else {
        bounds.y + OVERLAY_MARGIN
    };
    (x, y)
}

#[cfg(all(test, windows))]
mod overlay_state_tests {
    use super::*;

    fn game_window(foreground: bool) -> game::GameWindow {
        game::GameWindow {
            pid: 42,
            x: -2560,
            y: 0,
            width: 2560,
            height: 1440,
            foreground,
            minimized: false,
        }
    }

    fn hidden_step(detected: Option<game::GameWindow>, overlay_interactive: bool) -> GameWatchStep {
        GameWatchStep {
            detected,
            shell_mode: ShellMode::Tray,
            overlay_interactive,
            command: OverlayWindowCommand::HideToTray,
        }
    }

    #[test]
    fn game_absence_hides_to_tray_and_clears_interaction_mode() {
        let step = game_watch_step(
            ShellMode::OverlayInteractive,
            true,
            Some(game_window(true)),
            None,
            true,
        );

        assert_eq!(step, hidden_step(None, false));
    }

    #[test]
    fn minimized_or_invalid_game_bounds_are_treated_as_absent() {
        let visible = game_window(true);
        let minimized = game::GameWindow {
            minimized: true,
            ..visible
        };
        let zero_width = game::GameWindow {
            width: 0,
            ..visible
        };
        let zero_height = game::GameWindow {
            height: 0,
            ..visible
        };

        for raw_detected in [Some(minimized), Some(zero_width), Some(zero_height)] {
            let step = game_watch_step(
                ShellMode::OverlayPassive,
                false,
                Some(visible),
                raw_detected,
                false,
            );

            assert_eq!(step, hidden_step(None, false));
        }
    }

    #[test]
    fn visible_but_unfocused_game_hides_to_tray() {
        let unfocused = game_window(false);

        let step = game_watch_step(
            ShellMode::OverlayPassive,
            false,
            Some(unfocused),
            Some(unfocused),
            false,
        );

        assert_eq!(step, hidden_step(Some(unfocused), false));
    }

    #[test]
    fn repeated_unfocused_game_does_not_reapply_tray_hide() {
        let unfocused = game_window(false);

        let step = game_watch_step(
            ShellMode::Tray,
            false,
            Some(unfocused),
            Some(unfocused),
            false,
        );

        assert_eq!(
            step,
            GameWatchStep {
                detected: Some(unfocused),
                shell_mode: ShellMode::Tray,
                overlay_interactive: false,
                command: OverlayWindowCommand::None,
            }
        );
    }

    #[test]
    fn repeated_game_absence_does_not_reapply_tray_hide() {
        let step = game_watch_step(ShellMode::Tray, false, None, None, false);

        assert_eq!(
            step,
            GameWatchStep {
                detected: None,
                shell_mode: ShellMode::Tray,
                overlay_interactive: false,
                command: OverlayWindowCommand::None,
            }
        );
    }

    #[test]
    fn focused_game_opens_passive_overlay_when_not_typing() {
        let focused = game_window(true);

        let step = game_watch_step(ShellMode::Tray, false, None, Some(focused), false);

        assert_eq!(
            step,
            GameWatchStep {
                detected: Some(focused),
                shell_mode: ShellMode::OverlayPassive,
                overlay_interactive: false,
                command: OverlayWindowCommand::ApplyPassive(focused),
            }
        );
    }

    #[test]
    fn unchanged_focused_game_keeps_existing_passive_overlay() {
        let focused = game_window(true);

        let step = game_watch_step(
            ShellMode::OverlayPassive,
            false,
            Some(focused),
            Some(focused),
            false,
        );

        assert_eq!(
            step,
            GameWatchStep {
                detected: Some(focused),
                shell_mode: ShellMode::OverlayPassive,
                overlay_interactive: false,
                command: OverlayWindowCommand::None,
            }
        );
    }

    #[test]
    fn focused_game_bounds_change_reapplies_passive_overlay() {
        let previous = game_window(true);
        let resized = game::GameWindow {
            width: 1920,
            height: 1080,
            ..previous
        };

        let step = game_watch_step(
            ShellMode::OverlayPassive,
            false,
            Some(previous),
            Some(resized),
            false,
        );

        assert_eq!(
            step,
            GameWatchStep {
                detected: Some(resized),
                shell_mode: ShellMode::OverlayPassive,
                overlay_interactive: false,
                command: OverlayWindowCommand::ApplyPassive(resized),
            }
        );
    }

    #[test]
    fn typing_mode_opens_interactive_overlay_when_game_is_focused() {
        let focused = game_window(true);

        let step = game_watch_step(ShellMode::Tray, true, None, Some(focused), false);

        assert_eq!(
            step,
            GameWatchStep {
                detected: Some(focused),
                shell_mode: ShellMode::OverlayInteractive,
                overlay_interactive: true,
                command: OverlayWindowCommand::ApplyInteractive(focused),
            }
        );
    }

    #[test]
    fn typing_mode_stays_visible_when_overlay_itself_has_focus() {
        let unfocused_game = game_window(false);

        let step = game_watch_step(ShellMode::Tray, true, None, Some(unfocused_game), true);

        assert_eq!(
            step,
            GameWatchStep {
                detected: Some(unfocused_game),
                shell_mode: ShellMode::OverlayInteractive,
                overlay_interactive: true,
                command: OverlayWindowCommand::ApplyInteractive(unfocused_game),
            }
        );
    }

    #[test]
    fn typing_mode_hides_when_neither_game_nor_overlay_has_focus() {
        let unfocused_game = game_window(false);

        let step = game_watch_step(
            ShellMode::OverlayInteractive,
            true,
            Some(unfocused_game),
            Some(unfocused_game),
            false,
        );

        assert_eq!(step, hidden_step(Some(unfocused_game), true));
    }

    #[test]
    fn repeated_hidden_typing_mode_does_not_reapply_tray_hide() {
        let unfocused_game = game_window(false);

        let step = game_watch_step(
            ShellMode::Tray,
            true,
            Some(unfocused_game),
            Some(unfocused_game),
            false,
        );

        assert_eq!(
            step,
            GameWatchStep {
                detected: Some(unfocused_game),
                shell_mode: ShellMode::Tray,
                overlay_interactive: true,
                command: OverlayWindowCommand::None,
            }
        );
    }

    #[test]
    fn hotkey_open_requires_a_focused_visible_game_window() {
        let focused = game_window(true);
        let unfocused = game_window(false);
        let minimized = game::GameWindow {
            minimized: true,
            ..focused
        };
        let zero_width = game::GameWindow {
            width: 0,
            ..focused
        };
        let zero_height = game::GameWindow {
            height: 0,
            ..focused
        };

        assert_eq!(focused_visible_game_window(Some(focused)), Some(focused));
        for raw_detected in [
            None,
            Some(unfocused),
            Some(minimized),
            Some(zero_width),
            Some(zero_height),
        ] {
            assert_eq!(focused_visible_game_window(raw_detected), None);
        }
    }
}
