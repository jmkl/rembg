#![cfg_attr(windows, windows_subsystem = "windows")]
mod config;
mod register;
mod term;
use config::{Browser, Config};
use std::{
    io::{self, Write},
    process::ExitCode,
};
use url::Url;

const USAGE: &str = "LinkRouter

  linkrouter <url>         open a link (rules, default browser, or prompt)
  linkrouter --register    become the default http/https handler
  linkrouter --unregister  undo --register
  linkrouter --config      print the config file location";

fn main() -> ExitCode {
    term::init();

    let arg = std::env::args().nth(1);
    let is_url = matches!(arg.as_deref(), Some(a) if !a.starts_with('-'));
    if !is_url {
        term::ensure(); // CLI commands print, so make sure a console exists
    }

    let result = match arg.as_deref() {
        None | Some("--help") | Some("-h") => {
            println!("{USAGE}");
            Ok(())
        }
        Some("--register") => register::register().map(|m| println!("{m}")),
        Some("--unregister") => register::unregister().map(|m| println!("{m}")),
        Some("--config") => Config::path()
            .map(|p| println!("{}", p.display()))
            .ok_or_else(|| "no config directory on this system".to_string()),
        Some(raw) => open(raw),
    };

    let code = match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            term::ensure();
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    };
    if !is_url || code != ExitCode::SUCCESS {
        term::pause_if_owned();
    }
    code
}

fn open(raw: &str) -> Result<(), String> {
    // Only real web URLs are forwarded; this also prevents arguments like
    // "--some-flag" from ever reaching a browser.
    let url = Url::parse(raw)
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https"))
        .ok_or_else(|| format!("refusing to open: {raw}"))?;

    let cfg = Config::load()?;

    let direct = cfg
        .route(&url)
        .or_else(|| cfg.default_browser.as_deref().and_then(|n| cfg.browser(n)));

    let browser = match direct {
        Some(b) => b,
        None => match prompt(&url, &cfg.browsers)? {
            Some(b) => b,
            None => return Ok(()), // cancelled
        },
    };

    println!("-> {}: {}", browser.name, url);
    config::launch(browser, &url)
}

/// Console picker. Returns None if the user cancels.
fn prompt<'a>(url: &Url, browsers: &'a [Browser]) -> Result<Option<&'a Browser>, String> {
    term::own_console();
    println!("Open {url} with:\n");
    for (i, b) in browsers.iter().enumerate() {
        println!("  {}. {}", i + 1, b.name);
    }
    loop {
        print!("\nNumber (Enter to cancel): ");
        io::stdout().flush().map_err(|e| e.to_string())?;

        let mut line = String::new();
        let n = io::stdin()
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        let line = line.trim();
        if n == 0 || line.is_empty() || line.eq_ignore_ascii_case("q") {
            return Ok(None);
        }
        match line.parse::<usize>() {
            Ok(i) if (1..=browsers.len()).contains(&i) => return Ok(Some(&browsers[i - 1])),
            _ => println!("Enter a number between 1 and {}.", browsers.len()),
        }
    }
}
