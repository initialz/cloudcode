use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct Config {
    pub hub: HubConfig,
    #[serde(default)]
    pub agent: AgentSection,
    pub auth: AuthConfig,
    pub claude: ClaudeConfig,
}

#[derive(Debug, Deserialize)]
pub struct HubConfig {
    /// WebSocket URL of the hub, e.g. `wss://hub.example.com/v1/agent/ws`.
    pub url: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct AgentSection {
    /// Override the auto-generated agent name (`<hostname>-<user>`).
    /// Set this when the auto-generated name collides on the hub.
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AuthConfig {
    /// Plaintext secret presented to the hub in the `hello` frame. The hub
    /// stores its argon2id hash in `[[agents]].shared_secret_hash`.
    pub shared_secret: String,
}

#[derive(Debug, Deserialize)]
pub struct ClaudeConfig {
    /// Path to claude's credentials.json. Defaults to ~/.claude/.credentials.json.
    #[serde(default = "default_credentials_path")]
    pub credentials_path: PathBuf,

    /// Upstream Anthropic API base URL.
    #[serde(default = "default_upstream")]
    pub upstream: String,

    /// Anthropic-beta header values to send (joined with ',').
    #[serde(default = "default_anthropic_beta")]
    pub anthropic_beta: Vec<String>,
}

fn default_credentials_path() -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        home.join(".claude").join(".credentials.json")
    } else {
        PathBuf::from(".credentials.json")
    }
}

fn default_upstream() -> String {
    "https://api.anthropic.com".into()
}

fn default_anthropic_beta() -> Vec<String> {
    vec!["oauth-2025-04-20".into()]
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let s = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&s)?)
    }
}
