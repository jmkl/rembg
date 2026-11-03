//! Platform-specific window tweaks.

/// Removes the drop shadow and the 1 px frame line that winit/DWM draw around
/// undecorated windows. Idempotent, but touches window styles, so call it once.
#[cfg(windows)]
pub fn strip_window_chrome(window: &winit::window::Window) {
    use winit::platform::windows::{Color, WindowExtWindows};
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    const DWMWA_NCRENDERING_POLICY: u32 = 2;
    const DWMNCRP_DISABLED: i32 = 2;

    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: isize,
            attr: u32,
            value: *const core::ffi::c_void,
            size: u32,
        ) -> i32;
    }

    window.set_undecorated_shadow(false);
    // Match the border to the app background so any leftover DWM line blends in.
    window.set_border_color(Some(Color::from_rgb(18, 20, 26)));

    // Disable DWM non-client rendering: removes the last 1 px line Win11 draws
    // for windows that still carry WS_CAPTION (winit keeps it for snap support).
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(h) = handle.as_raw() else {
        return;
    };
    let policy = DWMNCRP_DISABLED;
    unsafe {
        DwmSetWindowAttribute(
            h.hwnd.get(),
            DWMWA_NCRENDERING_POLICY,
            &policy as *const i32 as *const core::ffi::c_void,
            size_of::<i32>() as u32,
        );
    }
}

#[cfg(not(windows))]
pub fn strip_window_chrome(_window: &winit::window::Window) {}
