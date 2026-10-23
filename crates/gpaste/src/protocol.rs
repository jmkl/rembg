use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    Hello {
        token: String,
    },
    SendPrompt {
        request_id: String,
        #[serde(default)]
        text: String,
        #[serde(default)]
        image_data_url: Option<String>,
        #[serde(default)]
        image_name: Option<String>,
    },
    PromptSent {
        request_id: String,
    },
    AssistantResponse {
        request_id: String,
        text: String,
    },
    BridgeStatus {
        connected: bool,
        detail: String,
    },
    BridgeError {
        #[serde(default)]
        request_id: Option<String>,
        error: String,
    },
    ImageCopied {
        request_id: String,
        base64: String,
        mime_type: String,
        width: u32,
        height: u32,
    },
}
