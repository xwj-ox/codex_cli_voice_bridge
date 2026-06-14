use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, ValueEnum};
use dialoguer::Input;

use codex_cli_voice_bridge_rust::config::{
    Credentials, MaiCredentials, default_credentials_path, default_mai_credentials_path,
    load_credentials, load_mai_credentials, save_credentials, save_mai_credentials,
};

#[derive(Debug, Parser)]
#[command(
    name = "configure_credentials",
    about = "Interactive editor for ASR credentials"
)]
struct Args {
    #[arg(long, value_enum, default_value_t = Provider::Doubao)]
    provider: Provider,
    #[arg(long, default_value = "")]
    config: String,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Provider {
    Doubao,
    Mai,
}

fn main() -> Result<()> {
    let args = Args::parse();

    match args.provider {
        Provider::Doubao => configure_doubao(&args.config),
        Provider::Mai => configure_mai(&args.config),
    }
}

fn configure_doubao(config: &str) -> Result<()> {
    let path = resolve_path(config, default_credentials_path);
    let existing = if path.is_file() {
        load_credentials(&path).unwrap_or_default()
    } else {
        Credentials::default()
    };
    println!("Credentials file: {}", path.display());

    let app_id: String = Input::new()
        .with_prompt("app_id")
        .with_initial_text(existing.app_id)
        .allow_empty(true)
        .interact_text()?;

    let access_token: String = Input::new()
        .with_prompt("access_token")
        .with_initial_text(existing.access_token)
        .allow_empty(true)
        .interact_text()?;

    let credentials = Credentials {
        app_id,
        access_token,
    }
    .normalized();

    save_credentials(&path, &credentials)?;
    println!("Saved credentials to {}", path.display());
    Ok(())
}

fn configure_mai(config: &str) -> Result<()> {
    let path = resolve_path(config, default_mai_credentials_path);
    let existing = if path.is_file() {
        load_mai_credentials(&path).unwrap_or_default()
    } else {
        MaiCredentials::default()
    };
    println!("Credentials file: {}", path.display());

    let endpoint: String = Input::new()
        .with_prompt("Azure Speech endpoint")
        .with_initial_text(existing.endpoint)
        .allow_empty(true)
        .interact_text()?;

    let key: String = Input::new()
        .with_prompt("Azure Speech key")
        .with_initial_text(existing.key)
        .allow_empty(true)
        .interact_text()?;

    let credentials = MaiCredentials { endpoint, key }.normalized();

    save_mai_credentials(&path, &credentials)?;
    println!("Saved credentials to {}", path.display());
    Ok(())
}

fn resolve_path(config: &str, default_path: impl FnOnce() -> PathBuf) -> PathBuf {
    if config.trim().is_empty() {
        default_path()
    } else {
        PathBuf::from(config.trim())
    }
}
