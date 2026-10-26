//! Persisted application settings.
//!
//! Settings live in the user's local app-data folder, not in a workspace, so
//! the workspace root can itself be changed from the settings window.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    /// Keep inference models in memory between runs. When disabled, each job
    /// loads the model it needs and releases it afterwards.
    pub cache_models: bool,
    /// Directory containing the ONNX models. `None` uses [`default_model_dir`].
    pub model_dir: Option<PathBuf>,
    /// Base folder for projects, exported images, masks and similar workspace
    /// data. Automatic sessions stay in app data.
    pub workspace_root: PathBuf,
    /// Canvas width in pixels. Default: 1280.
    pub canvas_width: f32,
    /// Canvas height in pixels. Default: 720.
    pub canvas_height: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            cache_models: true,
            model_dir: None,
            workspace_root: default_workspace_root(),
            canvas_width: 1280.0,
            canvas_height: 720.0,
        }
    }
}

impl Settings {
    pub(crate) fn load() -> Self {
        let path = settings_path();
        match fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(settings) => settings,
                Err(err) => {
                    eprintln!("compnode: invalid settings {}: {err}", path.display());
                    Self::default()
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(err) => {
                eprintln!("compnode: could not read {}: {err}", path.display());
                Self::default()
            }
        }
    }

    pub(crate) fn save(&self) -> Result<(), String> {
        let path = settings_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("create settings folder {}: {e}", parent.display()))?;
        }
        let text =
            serde_json::to_string_pretty(self).map_err(|e| format!("serialize settings: {e}"))?;
        fs::write(&path, format!("{text}\n"))
            .map_err(|e| format!("write settings {}: {e}", path.display()))
    }

    pub(crate) fn model_dir(&self) -> PathBuf {
        self.model_dir
            .as_ref()
            .filter(|path| !path.as_os_str().is_empty())
            .cloned()
            .unwrap_or_else(default_model_dir)
    }
    pub(crate) fn cache_dir(&self) -> PathBuf {
        let root = self.workspace_root().join("cache");
        _ = fs::create_dir_all(&root)
            .map_err(|e| format!("create workspace cache {}: {e}", root.display()));
        root
    }
    pub(crate) fn input_dir(&self) -> PathBuf {
        let root = self.workspace_root().join("input");
        _ = fs::create_dir_all(&root)
            .map_err(|e| format!("create workspace input {}: {e}", root.display()));
        root
    }
    pub(crate) fn autosave_path(&self) -> PathBuf {
        self.cache_dir().join("autosave.rembg.json")
    }

    pub(crate) fn workspace_root(&self) -> PathBuf {
        if self.workspace_root.as_os_str().is_empty() {
            default_workspace_root()
        } else {
            self.workspace_root.clone()
        }
    }

    /// Create the workspace root if it does not exist yet. Subfolders are
    /// created by the features that own them (projects, exports, masks, ...).
    pub(crate) fn ensure_workspace(&self) -> Result<(), String> {
        let root = self.workspace_root();
        fs::create_dir_all(&root)
            .map_err(|e| format!("create workspace root {}: {e}", root.display()))
    }
}

pub(crate) fn default_model_dir() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    cwd.join("models")
}

pub(crate) fn default_workspace_root() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let repo_workspaces = cwd.join("workspaces");
    if repo_workspaces.is_dir() {
        return repo_workspaces;
    }
    if let Some(base) = std::env::var_os("LOCALAPPDATA") {
        PathBuf::from(base).join("rembg").join("workspaces")
    } else {
        cwd.join("workspaces")
    }
}

pub(crate) fn settings_path() -> PathBuf {
    if let Some(base) = std::env::var_os("LOCALAPPDATA") {
        PathBuf::from(base)
            .join("rembg")
            .join("compnode-settings.json")
    } else {
        std::env::temp_dir().join("compnode-settings.json")
    }
}

/// Default directory for session file dialogs.
pub(crate) fn dialog_directory(settings: &Settings) -> PathBuf {
    let root = settings.workspace_root();
    if root.is_dir() {
        root
    } else {
        Path::new(".").to_path_buf()
    }
}
