use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;

const DEFAULT_CONFIG_FILE: &str = "quickstore_config.json";

#[derive(Serialize, Deserialize, Default)]
pub struct ConfigItem {
    prompts: Vec<String>,
}
impl ConfigItem {
    pub fn get(&self) -> &Vec<String> {
        &self.prompts
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
    pub fn add(&mut self, prompt: String) {
        self.item.prompts.push(prompt);
        if let Err(e) = self.save() {
            eprintln!("failed to save {}", e);
        }
    }
    pub fn remove(&mut self, index: usize) {
        if index < self.item.prompts.len() {
            self.item.prompts.remove(index);
            if let Err(e) = self.save() {
                eprintln!("failed to save {}", e);
            }
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
