use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use dialoguer::Input;

use codex_cli_voice_bridge_rust::config::{
    Credentials, default_credentials_path, load_credentials, save_credentials,
};

#[derive(Debug, Parser)]
#[command(name = "configure_credentials", about = "Interactive editor for doubao_credentials.json")]
struct Args {
    #[arg(long, default_value = "")]
    config: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let path = if args.config.trim().is_empty() {
        default_credentials_path()
    } else {
        PathBuf::from(args.config.trim())
    };

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
