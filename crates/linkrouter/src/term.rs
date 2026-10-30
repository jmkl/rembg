//! Console handling. On Windows the exe is a GUI-subsystem program (no window
//! when the OS launches it), and attaches to a console only when needed.
use std::sync::atomic::{AtomicBool, Ordering};

static HAS_CONSOLE: AtomicBool = AtomicBool::new(true);
static OWNED: AtomicBool = AtomicBool::new(false); // a console window we created

/// Call first thing in main.
pub fn init() {
    #[cfg(windows)]
    HAS_CONSOLE.store(win::attach_parent(), Ordering::Relaxed);
}

/// Call before printing anything. Opens a console window if there is none.
pub fn ensure() {
    #[cfg(windows)]
    if !HAS_CONSOLE.load(Ordering::Relaxed) && win::new_console() {
        HAS_CONSOLE.store(true, Ordering::Relaxed);
        OWNED.store(true, Ordering::Relaxed);
    }
}

/// For the interactive picker: always use a dedicated window, so we don't
/// fight the parent shell over stdin.
pub fn own_console() {
    #[cfg(windows)]
    if win::new_console() {
        HAS_CONSOLE.store(true, Ordering::Relaxed);
        OWNED.store(true, Ordering::Relaxed);
    }
}

/// Keeps a window we created open long enough to read the output.
pub fn pause_if_owned() {
    if OWNED.load(Ordering::Relaxed) {
        println!("\nPress Enter to close...");
        let _ = std::io::stdin().read_line(&mut String::new());
    }
}

#[cfg(windows)]
mod win {
    use std::{fs::OpenOptions, os::windows::io::IntoRawHandle};
    use windows_sys::Win32::System::Console::*;

    fn bound(which: STD_HANDLE) -> bool {
        let h = unsafe { GetStdHandle(which) } as isize;
        h != 0 && h != -1
    }

    /// Point std handles at the console. Without `force`, handles that already
    /// exist (e.g. `lr --config > out.txt`) are left alone.
    fn rebind(force: bool) {
        for (which, name, read) in [
            (STD_INPUT_HANDLE, "CONIN$", true),
            (STD_OUTPUT_HANDLE, "CONOUT$", false),
            (STD_ERROR_HANDLE, "CONOUT$", false),
        ] {
            if !force && bound(which) {
                continue;
            }
            if let Ok(f) = OpenOptions::new().read(read).write(!read).open(name) {
                unsafe { SetStdHandle(which, f.into_raw_handle() as _) };
            }
        }
    }

    pub fn attach_parent() -> bool {
        unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
        rebind(false);
        bound(STD_OUTPUT_HANDLE)
    }

    pub fn new_console() -> bool {
        unsafe {
            FreeConsole();
            AllocConsole();
        }
        rebind(true);
        bound(STD_OUTPUT_HANDLE)
    }
}
