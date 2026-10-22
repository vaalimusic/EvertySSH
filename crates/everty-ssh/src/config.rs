use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostConfig {
    pub id: String,
    pub name: String,
    pub hostname: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub username: String,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub badge_color: Option<String>,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub default_command: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

fn default_port() -> u16 {
    22
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthConfig {
    Agent,
    Key {
        path: PathBuf,
        #[serde(default)]
        passphrase: Option<String>,
    },
    Password {
        #[serde(default)]
        password: Option<String>,
    },
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self::Agent
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HostsDocument {
    #[serde(default)]
    pub settings: GlobalSettings,
    #[serde(default)]
    pub hosts: Vec<HostConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalSettings {
    #[serde(default = "default_font_size")]
    pub font_size: f32,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_scrollback")]
    pub scrollback_lines: usize,
    #[serde(default)]
    pub master_password_hash: Option<String>,
    #[serde(default = "default_confirm_delete")]
    pub confirm_delete: bool,
    #[serde(default = "default_user")]
    pub default_user: String,
}

impl Default for GlobalSettings {
    fn default() -> Self {
        Self {
            font_size: default_font_size(),
            theme: default_theme(),
            scrollback_lines: default_scrollback(),
            master_password_hash: None,
            confirm_delete: true,
            default_user: "root".to_string(),
        }
    }
}

fn default_font_size() -> f32 {
    14.0
}

fn default_theme() -> String {
    "dark".to_string()
}

fn default_scrollback() -> usize {
    5000
}

fn default_confirm_delete() -> bool {
    true
}

fn default_user() -> String {
    "root".to_string()
}

pub fn hash_password(password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"evertyssh_salt_v1_");
    hasher.update(password.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    hash_password(password) == hash
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportBundle {
    pub version: u32,
    pub exported_at: String,
    pub hosts: Vec<HostConfig>,
    #[serde(default)]
    pub settings: GlobalSettings,
}

pub fn config_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".evertyssh"))
        .unwrap_or_else(|| PathBuf::from(".evertyssh"))
}

pub fn hosts_file_path() -> PathBuf {
    config_dir().join("hosts.toml")
}

pub fn load_hosts() -> Result<HostsDocument, String> {
    let path = hosts_file_path();
    if !path.exists() {
        let sample = sample_hosts_document();
        let _ = save_hosts(&sample);
        return Ok(sample);
    }

    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;

    toml::from_str::<HostsDocument>(&content)
        .map_err(|e| format!("Failed to parse {}: {e}", path.display()))
}

pub fn save_hosts(doc: &HostsDocument) -> Result<(), String> {
    let dir = config_dir();
    fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create config dir {}: {e}", dir.display()))?;

    let path = hosts_file_path();
    let toml_str = toml::to_string_pretty(doc)
        .map_err(|e| format!("Failed to serialize TOML: {e}"))?;

    fs::write(&path, toml_str)
        .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;

    Ok(())
}

pub fn sample_hosts_document() -> HostsDocument {
    HostsDocument {
        settings: GlobalSettings::default(),
        hosts: vec![
            HostConfig {
                id: "local-dev".into(),
                name: "Local Development Server".into(),
                hostname: "127.0.0.1".into(),
                port: 22,
                username: "root".into(),
                group: Some("Development".into()),
                tags: vec!["local".into(), "docker".into()],
                badge_color: Some("#10b981".into()),
                auth: AuthConfig::Agent,
                default_command: None,
                env: HashMap::new(),
            },
            HostConfig {
                id: "prod-web".into(),
                name: "Production Web 01".into(),
                hostname: "web01.example.com".into(),
                port: 2222,
                username: "deploy".into(),
                group: Some("Production".into()),
                tags: vec!["prod".into(), "nginx".into()],
                badge_color: Some("#ef4444".into()),
                auth: AuthConfig::Key {
                    path: dirs::home_dir()
                        .map(|h| h.join(".ssh/id_ed25519"))
                        .unwrap_or_else(|| PathBuf::from("~/.ssh/id_ed25519")),
                    passphrase: None,
                },
                default_command: Some("tmux attach || tmux".into()),
                env: HashMap::new(),
            },
        ],
    }
}
