use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

pub const DEFAULT_CREDENTIALS_FILENAME: &str = "doubao_credentials.json";
pub const ENV_APP_ID: &str = "DOUBAO_ASR_APP_ID";
pub const ENV_ACCESS_TOKEN: &str = "DOUBAO_ASR_ACCESS_TOKEN";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Credentials {
    #[serde(default)]
    pub app_id: String,
    #[serde(default)]
    pub access_token: String,
}

impl Credentials {
    pub fn normalized(mut self) -> Self {
        self.app_id = self.app_id.trim().to_owned();
        self.access_token = self.access_token.trim().to_owned();
        self
    }

    pub fn validate(&self) -> Result<()> {
        if self.app_id.trim().is_empty() {
            bail!("Missing required value: app_id");
        }
        if self.access_token.trim().is_empty() {
            bail!("Missing required value: access_token");
        }
        Ok(())
    }
}

pub fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn runtime_root() -> PathBuf {
    if let Ok(current_dir) = env::current_dir() {
        return current_dir;
    }
    if let Ok(exe_path) = env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            return parent.to_path_buf();
        }
    }
    project_root()
}

pub fn default_credentials_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for path in [
        runtime_root().join(DEFAULT_CREDENTIALS_FILENAME),
        project_root().join(DEFAULT_CREDENTIALS_FILENAME),
    ] {
        if !candidates.iter().any(|existing| existing == &path) {
            candidates.push(path);
        }
    }
    candidates
}

pub fn default_credentials_path() -> PathBuf {
    default_credentials_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .unwrap_or_else(|| runtime_root().join(DEFAULT_CREDENTIALS_FILENAME))
}

pub fn load_credentials(path: &Path) -> Result<Credentials> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("Failed to read credentials file: {}", path.display()))?;
    let creds = serde_json::from_str::<Credentials>(&raw)
        .with_context(|| format!("Invalid credentials JSON: {}", path.display()))?;
    Ok(creds.normalized())
}

pub fn save_credentials(path: &Path, credentials: &Credentials) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }
    }
    let text = serde_json::to_string_pretty(&credentials.clone().normalized())?;
    fs::write(path, format!("{text}\n"))
        .with_context(|| format!("Failed to write credentials file: {}", path.display()))?;
    Ok(())
}

pub fn resolve_credentials(
    explicit_app_id: Option<&str>,
    explicit_access_token: Option<&str>,
    path: &Path,
) -> Result<Credentials> {
    let from_file = if path.is_file() {
        load_credentials(path)?
    } else {
        Credentials::default()
    };

    let env_app_id = env::var(ENV_APP_ID).unwrap_or_default();
    let env_access_token = env::var(ENV_ACCESS_TOKEN).unwrap_or_default();

    let credentials = Credentials {
        app_id: first_nonempty(&[
            explicit_app_id.unwrap_or(""),
            env_app_id.as_str(),
            from_file.app_id.as_str(),
        ]),
        access_token: first_nonempty(&[
            explicit_access_token.unwrap_or(""),
            env_access_token.as_str(),
            from_file.access_token.as_str(),
        ]),
    }
    .normalized();

    if credentials.app_id.is_empty() || credentials.access_token.is_empty() {
        return Err(anyhow!(
            "Credentials are incomplete. Use doubao_credentials.json, env vars, or CLI arguments."
        ));
    }
    Ok(credentials)
}

fn first_nonempty(values: &[&str]) -> String {
    values
        .iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .unwrap_or("")
        .to_owned()
}
