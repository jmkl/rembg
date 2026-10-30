//! Registers / unregisters LinkRouter as an http/https handler.

#[cfg(windows)]
pub fn register() -> Result<String, String> {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let e = |e: std::io::Error| e.to_string();

    let (k, _) = hkcu
        .create_subkey(r"Software\Classes\LinkRouterURL")
        .map_err(e)?;
    k.set_value("", &"LinkRouter URL").map_err(e)?;
    k.set_value("URL Protocol", &"").map_err(e)?;

    let (k, _) = hkcu
        .create_subkey(r"Software\Classes\LinkRouterURL\shell\open\command")
        .map_err(e)?;
    k.set_value("", &format!("\"{}\" \"%1\"", exe.display()))
        .map_err(e)?;

    let (k, _) = hkcu
        .create_subkey(r"Software\LinkRouter\Capabilities")
        .map_err(e)?;
    k.set_value("ApplicationName", &"LinkRouter").map_err(e)?;
    k.set_value(
        "ApplicationDescription",
        &"Routes links to the browser of your choice",
    )
    .map_err(e)?;

    let (k, _) = hkcu
        .create_subkey(r"Software\LinkRouter\Capabilities\URLAssociations")
        .map_err(e)?;
    k.set_value("http", &"LinkRouterURL").map_err(e)?;
    k.set_value("https", &"LinkRouterURL").map_err(e)?;

    let (k, _) = hkcu
        .create_subkey(r"Software\RegisteredApplications")
        .map_err(e)?;
    k.set_value("LinkRouter", &r"Software\LinkRouter\Capabilities")
        .map_err(e)?;

    // Windows does not allow silently becoming the default; open the settings page.
    let _ = std::process::Command::new("explorer.exe")
        .arg("ms-settings:defaultapps")
        .spawn();

    Ok(
        "Registered.\n\nIn the Default apps page that just opened, search for \
        LinkRouter and set it for HTTP and HTTPS."
            .into(),
    )
}

#[cfg(windows)]
pub fn unregister() -> Result<String, String> {
    use winreg::{
        RegKey,
        enums::{HKEY_CURRENT_USER, KEY_WRITE},
    };
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let _ = hkcu.delete_subkey_all(r"Software\Classes\LinkRouterURL");
    let _ = hkcu.delete_subkey_all(r"Software\LinkRouter");
    if let Ok(k) = hkcu.open_subkey_with_flags(r"Software\RegisteredApplications", KEY_WRITE) {
        let _ = k.delete_value("LinkRouter");
    }
    Ok("Unregistered. Pick another default browser in Windows Settings.".into())
}

#[cfg(target_os = "linux")]
fn desktop_file() -> Result<std::path::PathBuf, String> {
    dirs::data_dir()
        .map(|d| d.join("applications").join("linkrouter.desktop"))
        .ok_or_else(|| "no data directory".to_string())
}

#[cfg(target_os = "linux")]
pub fn register() -> Result<String, String> {
    use std::{fs, process::Command};

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let path = desktop_file()?;
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=LinkRouter\nComment=Routes links to the browser of your choice\n\
         Exec=\"{}\" %u\nTerminal=false\nCategories=Network;WebBrowser;\n\
         MimeType=x-scheme-handler/http;x-scheme-handler/https;\n",
        exe.display()
    );
    fs::write(&path, entry).map_err(|e| e.to_string())?;

    let _ = Command::new("update-desktop-database")
        .arg(path.parent().unwrap())
        .status();
    let ok = Command::new("xdg-settings")
        .args(["set", "default-web-browser", "linkrouter.desktop"])
        .status()
        .map_or(false, |s| s.success());
    for scheme in ["http", "https"] {
        let _ = Command::new("xdg-mime")
            .args([
                "default",
                "linkrouter.desktop",
                &format!("x-scheme-handler/{scheme}"),
            ])
            .status();
    }
    Ok(if ok {
        "Registered as default browser.".into()
    } else {
        "Desktop entry installed, but xdg-settings failed; set LinkRouter as default \
         in your desktop's settings."
            .into()
    })
}

#[cfg(target_os = "linux")]
pub fn unregister() -> Result<String, String> {
    let path = desktop_file()?;
    let _ = std::fs::remove_file(path);
    Ok("Removed. Pick another default browser in your desktop settings.".into())
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn register() -> Result<String, String> {
    Err(
        "Registration is not implemented on this OS (macOS needs an .app bundle \
         with CFBundleURLTypes)."
            .into(),
    )
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn unregister() -> Result<String, String> {
    register()
}
