# LinkRouter (`lr`)

A tiny link router written in Rust. Set it as your default web browser, and it
forwards every `http`/`https` link to the real browser (and profile) you want,
based on rules you write in a simple TOML file.

- **Rules by host and path:** GitHub in Firefox, YouTube in Chrome, work links in a specific profile.
- **Profiles and flags:** pass any arguments to a browser, such as `-P work` or `--profile-directory`.
- **Default browser:** anything without a rule goes to the browser you choose, or you pick from a console prompt.
- **Self-registering:** `lr --register` sets up the OS integration for you.
- **No GUI, no runtime:** a single small binary with a plain-text config.

## Install

```sh
cargo build --release
```

The binary is `target/release/lr` (`lr.exe` on Windows). Copy it somewhere
permanent first, because registration stores the path to the exe.

## Quick start

```sh
lr --register      # make lr the default http/https handler
lr --config        # print the config file location, then edit it
lr https://github.com/rust-lang/rust   # try it manually
```

- **Windows:** `--register` writes the registry entries and opens _Settings → Default apps_. Windows does not allow apps to set themselves as default silently, so choose **LinkRouter** for HTTP and HTTPS there.
- **Linux:** `--register` installs a `.desktop` file and runs `xdg-settings` and `xdg-mime`.
- **macOS:** not supported yet. It needs an `.app` bundle with `CFBundleURLTypes`.

To undo it, run `lr --unregister` and pick another default browser in your OS settings.

## Configuration

On first run, a starter config is created at:

| OS      | Location                           |
| ------- | ---------------------------------- |
| Windows | `%APPDATA%\linkrouter\config.toml` |
| Linux   | `~/.config/linkrouter/config.toml` |

```toml
# Used when no rule matches. Remove this line to get the picker instead.
# Must stay above the first [[browsers]] section.
default_browser = "Firefox"

[[browsers]]
name = "Firefox"
path = 'C:\Program Files\Mozilla Firefox\firefox.exe'

[[browsers]]
name = "Firefox (work)"
path = 'C:\Program Files\Mozilla Firefox\firefox.exe'
args = ["-P", "work"]

[[browsers]]
name = "Chrome (Profile 1)"
path = 'C:\Program Files\Google\Chrome\Application\chrome.exe'
args = ["--profile-directory=Profile 1"]

# First matching rule wins.
[[rules]]
host = "github.com"
path_prefix = "/my-company"
browser = "Chrome (Profile 1)"

[[rules]]
host = "github.com"
browser = "Firefox"
```

### Browsers

| Key    | Description                                                                                                            |
| ------ | ---------------------------------------------------------------------------------------------------------------------- |
| `name` | Name used by rules and `default_browser` (case-insensitive).                                                           |
| `path` | Executable path, or a command on `PATH` (Linux). Use single quotes on Windows so backslashes stay literal.             |
| `args` | Optional extra arguments. `{url}` is replaced by the link. If no argument contains `{url}`, the link is appended last. |

### Rules

| Key           | Description                                                                       |
| ------------- | --------------------------------------------------------------------------------- |
| `host`        | Matches the host and all its subdomains (`github.com` matches `gist.github.com`). |
| `path_prefix` | Optional. The URL path must start with this.                                      |
| `browser`     | Name of a browser from `[[browsers]]`.                                            |

Rules are matched against the parsed host, not a substring of the URL, so
`https://evil.com/?github.com` does not match `github.com`.

### Order of decisions

1. The first matching rule picks the browser.
2. Otherwise `default_browser` is used, if set.
3. Otherwise a numbered list is printed and you choose (Enter cancels).

## Commands

```text
lr <url>          open a link
lr --register     become the default http/https handler
lr --unregister   undo --register
lr --config       print the config file location
lr --help         show usage
```

## Security notes

- Only `http` and `https` URLs are accepted, so an argument like `--some-flag` can never reach a browser.
- The browser is started directly, never through `open`, `xdg-open` or `start`, which would resolve back to LinkRouter and loop. A guard also refuses a browser entry that points to LinkRouter itself.
- The link is passed as a separate argument, never through a shell.

## Troubleshooting

- **Nothing happens, or the wrong browser opens:** check the exact `name` spelling in your rules. Run `lr <url>` from a terminal to see errors.
- **Config error:** `default_browser` placed after a `[[browsers]]` line becomes part of that entry and is ignored.
- **Links loop or open LinkRouter again:** a browser `path` must point to the real browser executable.
- **Windows asks which app to use every time:** re-run `lr --register` after moving the exe, then reselect it in Default apps.

## License

[MIT](LICENSE)
