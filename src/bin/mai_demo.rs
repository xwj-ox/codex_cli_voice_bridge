use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};

use codex_cli_voice_bridge_rust::audio::{MicrophoneCaptureOptions, list_input_devices};
use codex_cli_voice_bridge_rust::config::{default_mai_credentials_path, resolve_mai_credentials};
use codex_cli_voice_bridge_rust::mai::{
    DEFAULT_MAI_API_VERSION, DEFAULT_MAI_MODEL, MaiOptions, guess_audio_content_type,
    run_file_session, run_mic_session,
};

const MIC_UPLOAD_SAMPLE_RATE: u32 = 16000;
const MIC_UPLOAD_CHANNELS: u16 = 1;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum InputSource {
    File,
    Mic,
}

#[derive(Debug, Parser)]
#[command(
    name = "mai_demo",
    about = "MAI demo",
    long_about = "Run MAI against an audio file or a fixed-duration microphone capture, then print or save the recognition result.",
    next_line_help = true
)]
struct Args {
    #[arg(long, value_enum, default_value_t = InputSource::File, help = "Input source: file reads a local audio file, mic records a fixed-duration utterance")]
    input_source: InputSource,
    #[arg(
        long,
        default_value = "",
        help = "Path to the local audio file; required when --input-source file"
    )]
    audio_file: String,
    #[arg(long, default_value = "", help = "Azure Speech endpoint")]
    endpoint: String,
    #[arg(long, default_value = "", help = "Azure Speech key")]
    key: String,
    #[arg(
        long,
        default_value = "",
        help = "Path to mai_credentials.json; defaults to the current working directory"
    )]
    credentials: String,
    #[arg(long, default_value = DEFAULT_MAI_API_VERSION, help = "Azure Speech API version")]
    api_version: String,
    #[arg(long, default_value = DEFAULT_MAI_MODEL, help = "MAI model: mai-transcribe-1 or mai-transcribe-1.5")]
    model: String,
    #[arg(
        long,
        value_delimiter = ',',
        default_value = "zh",
        help = "MAI locale hint; repeat or comma-separate values"
    )]
    locale: Vec<String>,
    #[arg(
        long,
        default_value = "default",
        help = "MAI style: default or verbatim"
    )]
    style: String,
    #[arg(
        long,
        help = "MAI 1.5 phrase list entry; repeat to add multiple phrases"
    )]
    phrase: Vec<String>,
    #[arg(
        long,
        default_value_t = 10.0,
        help = "Microphone recording duration in seconds when --input-source mic"
    )]
    mic_duration: f32,
    #[arg(
        long,
        default_value = "",
        help = "Microphone selector: device index or a case-insensitive name substring"
    )]
    mic_device: String,
    #[arg(long, help = "List available input devices and exit")]
    mic_list_devices: bool,
    #[arg(
        long,
        default_value_t = 200,
        help = "Microphone capture chunk size before local buffering"
    )]
    chunk_ms: u32,
    #[arg(
        long,
        default_value_t = 0,
        help = "Optional fixed microphone chunk size in bytes; 0 means derive from chunk_ms"
    )]
    chunk_bytes: usize,
    #[arg(
        long,
        default_value_t = 600.0,
        help = "HTTP request timeout in seconds"
    )]
    timeout: f64,
    #[arg(
        long,
        default_value_t = 4,
        help = "Retry count for 429 and transient 5xx responses"
    )]
    max_retries: usize,
    #[arg(
        long,
        default_value = "",
        help = "Write the raw final JSON result to this path; empty disables file output"
    )]
    output_json: String,
    #[arg(
        long,
        default_value_t = false,
        help = "Reduce console output to only essential lines"
    )]
    quiet: bool,
}

fn save_result_json(path: &str, result: &serde_json::Value) -> Result<()> {
    if path.trim().is_empty() {
        return Ok(());
    }
    let target_path = PathBuf::from(path);
    if let Some(parent) = target_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create output directory: {}", parent.display())
            })?;
        }
    }
    fs::write(
        &target_path,
        format!("{}\n", serde_json::to_string_pretty(result)?),
    )
    .with_context(|| format!("Failed to write output JSON: {}", target_path.display()))?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    if args.mic_list_devices {
        let devices = list_input_devices()?;
        if devices.is_empty() {
            println!("No input devices found.");
            return Ok(());
        }
        for device in devices {
            let marker = if device.is_default_input {
                " (default)"
            } else {
                ""
            };
            println!(
                "[{}] {}{} | {} Hz / {} ch / {}",
                device.index,
                device.name,
                marker,
                device.default_sample_rate,
                device.channels,
                device.sample_format
            );
        }
        return Ok(());
    }

    let credentials_path = if args.credentials.trim().is_empty() {
        default_mai_credentials_path()
    } else {
        PathBuf::from(args.credentials.trim())
    };
    let credentials =
        resolve_mai_credentials(Some(&args.endpoint), Some(&args.key), &credentials_path)?;

    let options = MaiOptions {
        endpoint: credentials.endpoint,
        key: credentials.key,
        api_version: args.api_version.trim().to_owned(),
        model: args.model.trim().to_owned(),
        locales: args.locale.clone(),
        style: args.style.trim().to_owned(),
        phrases: args
            .phrase
            .iter()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .collect(),
        timeout_seconds: args.timeout,
        max_retries: args.max_retries,
    };

    let result = match args.input_source {
        InputSource::File => {
            if args.audio_file.trim().is_empty() {
                bail!("--audio-file is required when --input-source file");
            }
            let path = PathBuf::from(args.audio_file.trim());
            let audio_bytes = fs::read(&path)
                .with_context(|| format!("Failed to read audio file: {}", path.display()))?;
            if audio_bytes.is_empty() {
                bail!("Audio file is empty: {}", path.display());
            }
            if !args.quiet {
                println!("Provider: MAI");
                println!("Input source: file");
                println!("Audio bytes: {}", audio_bytes.len());
                println!("Model: {}", options.model);
            }
            let file_name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("audio");
            run_file_session(
                &options,
                &audio_bytes,
                file_name,
                guess_audio_content_type(&path),
            )
            .await?
        }
        InputSource::Mic => {
            if !args.quiet {
                println!("Provider: MAI");
                println!("Input source: mic");
                println!("Model: {}", options.model);
                println!(
                    "Microphone capture: duration={:.1}s selector={}",
                    args.mic_duration,
                    if args.mic_device.trim().is_empty() {
                        "<default>"
                    } else {
                        args.mic_device.trim()
                    }
                );
            }
            let capture_options = MicrophoneCaptureOptions {
                duration_seconds: args.mic_duration,
                device_selector: (!args.mic_device.trim().is_empty())
                    .then(|| args.mic_device.trim().to_owned()),
                chunk_ms: args.chunk_ms,
                chunk_bytes: args.chunk_bytes,
                stop_signal: None,
                target_sample_rate: MIC_UPLOAD_SAMPLE_RATE,
                target_channels: MIC_UPLOAD_CHANNELS,
            };
            run_mic_session(&options, &capture_options).await?
        }
    };

    println!("ASR Final Text: {}", result.final_text.trim());
    save_result_json(&args.output_json, &result.response_json)?;
    Ok(())
}
