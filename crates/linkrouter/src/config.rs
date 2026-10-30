use serde::Deserialize;
use std::{fs, path::PathBuf, process::Command};
use url::Url;

#[derive(Debug, Deserialize)]
pub struct Config {
    /// Name of a browser to use when no rule matches. If unset, the picker is shown.
    pub default_browser: Option<String>,
    #[serde(default)]
    pub browsers: Vec<Browser>,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Browser {
    pub name: String,
    pub path: String,
    /// Extra arguments (profiles, flags). `{url}` is replaced by the link;
    /// if no argument contains `{url}`, the link is appended last.
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct Rule {
    /// Matches this host and all its subdomains ("github.com" matches "gist.github.com").
    pub host: String,
    /// Optional: URL path must start with this (e.g. "/myorg").
    pub path_prefix: Option<String>,
    /// Name of a browser from `[[browsers]]`.
    pub browser: String,
}

impl Rule {
    fn matches(&self, host: &str, path: &str) -> bool {
        let want = self.host.trim_start_matches("*.").to_ascii_lowercase();
        let host_ok = host == want || host.ends_with(&format!(".{want}"));
        host_ok
            && self
                .path_prefix
                .as_deref()
                .map_or(true, |p| path.starts_with(p))
    }
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("linkrouter").join("config.toml"))
    }

    /// Loads the config, creating a starter file on first run.
    pub fn load() -> Result<Config, String> {
        let path = Self::path().ok_or("no config directory on this system")?;
        if !path.exists() {
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            fs::write(&path, DEFAULT_CONFIG)
                .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        }
        let text = fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let cfg: Config = toml::from_str(&text).map_err(|e| format!("{}:\n{e}", path.display()))?;
        if cfg.browsers.is_empty() {
            return Err(format!("No [[browsers]] defined in {}", path.display()));
        }
        Ok(cfg)
    }

    pub fn browser(&self, name: &str) -> Option<&Browser> {
        self.browsers
            .iter()
            .find(|b| b.name.eq_ignore_ascii_case(name))
    }

    /// First matching rule wins. Matching uses the parsed host, never a substring
    /// of the whole URL, so "evil.com/?github.com" does not match "github.com".
    pub fn route(&self, url: &Url) -> Option<&Browser> {
        let host = url.host_str()?.to_ascii_lowercase();
        let rule = self.rules.iter().find(|r| r.matches(&host, url.path()))?;
        self.browser(&rule.browser)
    }
}

/// Starts the browser executable directly (never via the OS "open" handler,
/// which would resolve back to us and loop forever).
pub fn launch(browser: &Browser, url: &Url) -> Result<(), String> {
    // Loop guard: refuse to launch ourselves.
    if let (Ok(me), Ok(target)) = (std::env::current_exe(), fs::canonicalize(&browser.path)) {
        if fs::canonicalize(me).map_or(false, |me| me == target) {
            return Err(format!(
                "browser '{}' points to LinkRouter itself",
                browser.name
            ));
        }
    }

    let mut args = Vec::new();
    let mut has_placeholder = false;
    for a in &browser.args {
        if a.contains("{url}") {
            has_placeholder = true;
            args.push(a.replace("{url}", url.as_str()));
        } else {
            args.push(a.clone());
        }
    }
    if !has_placeholder {
        args.push(url.to_string());
    }

    Command::new(&browser.path)
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("{}: {e}", browser.path))
}

#[cfg(windows)]
const DEFAULT_CONFIG: &str = r#"# LinkRouter config
# default_browser = "Firefox"   # skip the picker when no rule matches

[[browsers]]
name = "Firefox"
path = 'C:\Program Files\Mozilla Firefox\firefox.exe'

[[browsers]]
name = "Firefox (work)"
path = 'C:\Program Files\Mozilla Firefox\firefox.exe'
args = ["-P", "work"]

[[browsers]]
name = "Chrome"
path = 'C:\Program Files\Google\Chrome\Application\chrome.exe'

[[browsers]]
name = "Chrome (Profile 1)"
path = 'C:\Program Files\Google\Chrome\Application\chrome.exe'
args = ["--profile-directory=Profile 1"]

[[browsers]]
name = "Edge"
path = 'C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe'

# First matching rule wins. `host` also matches subdomains.
[[rules]]
host = "github.com"
browser = "Firefox"

[[rules]]
host = "github.com"
path_prefix = "/my-company"
browser = "Chrome (Profile 1)"

[[rules]]
host = "youtube.com"
browser = "Chrome"
"#;

#[cfg(not(windows))]
const DEFAULT_CONFIG: &str = r#"# LinkRouter config
# default_browser = "Firefox"   # skip the picker when no rule matches
# Use absolute paths, or names resolvable via PATH.

[[browsers]]
name = "Firefox"
path = "firefox"

[[browsers]]
name = "Firefox (work)"
path = "firefox"
args = ["-P", "work"]

[[browsers]]
name = "Chrome"
path = "google-chrome"

[[browsers]]
name = "Chrome (Profile 1)"
path = "google-chrome"
args = ["--profile-directory=Profile 1"]

# First matching rule wins. `host` also matches subdomains.
[[rules]]
host = "github.com"
browser = "Firefox"

[[rules]]
host = "youtube.com"
browser = "Chrome"
"#;
