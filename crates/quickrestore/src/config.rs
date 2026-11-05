use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;

const DEFAULT_CONFIG_FILE: &str = "quickstore_config.json";

/// Which model runs the face-restore stage.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FaceRestoreModel {
    #[default]
    Gfpgan,
    Gpen,
}

impl FaceRestoreModel {
    pub const ALL: [Self; 2] = [Self::Gfpgan, Self::Gpen];

    pub fn label(self) -> &'static str {
        match self {
            Self::Gfpgan => "GFPGAN v1.4",
            Self::Gpen => "GPEN-BFR-512",
        }
    }
}

/// Which model removes the background at the end of the pipeline.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundRemovalModel {
    #[default]
    Rmbg,
    Birefnet,
}

impl BackgroundRemovalModel {
    pub const ALL: [Self; 2] = [Self::Rmbg, Self::Birefnet];

    pub fn label(self) -> &'static str {
        match self {
            Self::Rmbg => "RMBG 1.4",
            Self::Birefnet => "BiRefNet",
        }
    }
}

/// Snapshot of the settings that control the server pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperationOptions {
    pub face_restore: FaceRestoreModel,
    pub background_removal: BackgroundRemovalModel,
    pub upscale: bool,
}

impl Default for OperationOptions {
    fn default() -> Self {
        Self {
            face_restore: FaceRestoreModel::default(),
            background_removal: BackgroundRemovalModel::default(),
            upscale: true,
        }
    }
}

fn default_upscale() -> bool {
    true
}

#[derive(Serialize, Deserialize)]
pub struct ConfigItem {
    prompts: Vec<String>,
    #[serde(default)]
    face_restore: FaceRestoreModel,
    #[serde(default)]
    background_removal: BackgroundRemovalModel,
    #[serde(default = "default_upscale")]
    upscale: bool,
}

impl Default for ConfigItem {
    fn default() -> Self {
        Self {
            prompts: Vec::new(),
            face_restore: FaceRestoreModel::default(),
            background_removal: BackgroundRemovalModel::default(),
            upscale: default_upscale(),
        }
    }
}

impl ConfigItem {
    pub fn get(&self) -> &Vec<String> {
        &self.prompts
    }
    pub fn face_restore(&self) -> FaceRestoreModel {
        self.face_restore
    }
    pub fn background_removal(&self) -> BackgroundRemovalModel {
        self.background_removal
    }
    pub fn upscale(&self) -> bool {
        self.upscale
    }
}

pub struct Config {
    item: ConfigItem,
    path: String,
}

impl Config {
    pub fn new() -> Self {
        match Config::load_file() {
            Ok((path, item)) => Self { item, path },
            Err(e) => {
                eprintln!("config load failed: {e}");
                let path = Config::root_dir().join(DEFAULT_CONFIG_FILE);
                Self {
                    item: ConfigItem::default(),
                    path: path.to_string_lossy().into(),
                }
            }
        }
    }
    pub fn load(&self) -> &ConfigItem {
        &self.item
    }
    pub fn options(&self) -> OperationOptions {
        OperationOptions {
            face_restore: self.item.face_restore,
            background_removal: self.item.background_removal,
            upscale: self.item.upscale,
        }
    }
    pub fn set_face_restore(&mut self, model: FaceRestoreModel) {
        if self.item.face_restore != model {
            self.item.face_restore = model;
            self.save_logged();
        }
    }
    pub fn set_background_removal(&mut self, model: BackgroundRemovalModel) {
        if self.item.background_removal != model {
            self.item.background_removal = model;
            self.save_logged();
        }
    }
    pub fn set_upscale(&mut self, upscale: bool) {
        if self.item.upscale != upscale {
            self.item.upscale = upscale;
            self.save_logged();
        }
    }
    pub fn add(&mut self, prompt: String) {
        self.item.prompts.push(prompt);
        self.save_logged();
    }
    pub fn remove(&mut self, index: usize) {
        if index < self.item.prompts.len() {
            self.item.prompts.remove(index);
            self.save_logged();
        }
    }
    fn save_logged(&mut self) {
        if let Err(e) = self.save() {
            eprintln!("failed to save {}", e);
        }
    }
    fn save(&mut self) -> anyhow::Result<()> {
        let content = serde_json::to_string_pretty(&self.item)?;
        fs::write(&self.path, &content)?;
        Ok(())
    }
    fn load_file() -> anyhow::Result<(String, ConfigItem)> {
        let path = Config::root_dir().join(DEFAULT_CONFIG_FILE);
        let result = if !&path.exists() {
            let default = ConfigItem::default();
            let content = serde_json::to_string_pretty(&default)?;
            std::fs::write(&path, content)?;
            default
        } else {
            let text = fs::read_to_string(&path)?;
            let result = serde_json::from_str::<ConfigItem>(&text)?;
            result
        };
        Ok((path.to_string_lossy().to_string(), result))
    }
    pub fn root_dir() -> PathBuf {
        let output_dir = std::env::var("LOCALAPPDATA")
            .map(|p| {
                std::path::PathBuf::from(p)
                    .join("QuickRestore")
                    .join("output")
            })
            .unwrap_or_else(|_| std::env::temp_dir().join("quickrestore"));
        let _ = std::fs::create_dir_all(&output_dir);
        output_dir
    }
}
