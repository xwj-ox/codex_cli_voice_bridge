use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use serde_json::json;
use uuid::Uuid;

use codex_cli_voice_bridge_rust::asr::{SessionOptions, run_file_session, run_mic_session};
use codex_cli_voice_bridge_rust::audio::{MicrophoneCaptureOptions, list_input_devices};
use codex_cli_voice_bridge_rust::config::{default_credentials_path, resolve_credentials};
use codex_cli_voice_bridge_rust::protocol::{DEFAULT_RESOURCE_ID, DEFAULT_WS_URL};

const MIC_UPLOAD_SAMPLE_RATE: u32 = 16000;
const MIC_UPLOAD_BITS: u16 = 16;
const MIC_UPLOAD_CHANNELS: u16 = 1;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum InputSource {
    File,
    Mic,
}

#[derive(Debug, Parser)]
#[command(
    name = "doubao_asr_demo",
    about = "Rust port of the Doubao ASR demo",
    long_about = "Run Doubao ASR against an audio file or a fixed-duration microphone capture, then print or save the recognition result.",
    next_line_help = true
)]
struct Args {
    #[arg(long, value_enum, default_value_t = InputSource::File, help = "Input source: file reads a local audio file, mic records a fixed-duration utterance")]
    input_source: InputSource,
    #[arg(long, default_value = "", help = "Path to the local audio file; required when --input-source file")]
    audio_file: String,
    #[arg(long, default_value = "pcm", help = "Input audio format. The docs use raw/pcm for microphone-style uploads")]
    audio_format: String,
    #[arg(long, default_value = "", help = "Doubao app_id; falls back to the credentials file when omitted")]
    app_id: String,
    #[arg(long, default_value = "", help = "Doubao access_token; falls back to the credentials file when omitted")]
    access_token: String,
    #[arg(long, default_value = DEFAULT_RESOURCE_ID, help = "Doubao ASR resource_id; docs list volc.bigasr.sauc.duration/concurrent for 1.0 and volc.seedasr.sauc.duration/concurrent for 2.0")]
    resource_id: String,
    #[arg(long, default_value = DEFAULT_WS_URL, help = "Doubao WebSocket URL; default is the optimized bidirectional endpoint /api/v3/sauc/bigmodel_async")]
    ws_url: String,
    #[arg(long, default_value = "", help = "Optional connect_id for tracing a session end-to-end")]
    connect_id: String,
    #[arg(long, default_value = "", help = "User ID sent in the request; a random one is generated when omitted")]
    uid: String,
    #[arg(long, default_value_t = 16000, help = "Input sample rate in Hz for file mode; mic mode always uploads 16000 Hz after conversion")]
    sample_rate: u32,
    #[arg(long, default_value_t = 16, help = "Input bit depth for file mode; mic mode always uploads 16-bit PCM after conversion")]
    bits: u16,
    #[arg(long, default_value_t = 1, help = "Input channel count for file mode; mic mode always uploads mono after conversion")]
    channels: u16,
    #[arg(long, default_value = "", help = "Optional language hint; official docs say it is only supported by bigmodel_nostream and not by the second-pass recognition path")]
    language: String,
    #[arg(long, default_value_t = 10.0, help = "Microphone recording duration in seconds when --input-source mic")]
    mic_duration: f32,
    #[arg(long, default_value = "", help = "Microphone selector: device index or a case-insensitive name substring")]
    mic_device: String,
    #[arg(long, help = "List available input devices and exit")]
    mic_list_devices: bool,
    #[arg(long, default_value_t = 200, help = "Audio chunk size before upload; official docs recommend about 100-200 ms, and 200 ms is preferred for bigmodel_async")]
    chunk_ms: u32,
    #[arg(long, default_value_t = 0, help = "Optional fixed chunk size in bytes; 0 means derive from chunk_ms")]
    chunk_bytes: usize,
    #[arg(long, default_value_t = 200, help = "Delay between sending audio chunks in milliseconds; official docs recommend about 100-200 ms and warn against values that are too large or too small")]
    send_interval_ms: u64,
    #[arg(long, default_value = "bigmodel", help = "Doubao ASR model name")]
    model_name: String,
    #[arg(long, default_value_t = true, help = "Enable ITN text normalization, e.g. turning spoken numerals into written forms like year 1970 or amount $123")]
    enable_itn: bool,
    #[arg(long, default_value_t = true, help = "Enable punctuation insertion; official docs say this defaults to true")]
    enable_punc: bool,
    #[arg(long, default_value_t = false, help = "Enable semantic smoothing (DDC, likely Disfluency Detection and Correction), which removes fillers, hesitations, and repeated words to improve readability; official docs say this defaults to false")]
    enable_ddc: bool,
    #[arg(long, default_value_t = false, help = "Enable second-pass recognition: on the optimized bidirectional API, each finalized VAD segment is re-recognized with the nostream model to improve final accuracy")]
    enable_nonstream: bool,
    #[arg(long, default_value_t = true, help = "Return utterance-level segmentation details such as pauses, sentence splits, and word information")]
    show_utterances: bool,
    #[arg(long, default_value = "full", help = "Result return mode: full returns all utterances each time; single returns only the current utterance and is intended to be used with show_utterances=true")]
    result_type: String,
    #[arg(long, default_value_t = 10.0, help = "Handshake and per-frame timeout in seconds")]
    timeout: f64,
    #[arg(long, default_value_t = 15.0, help = "Final result wait timeout in seconds after audio upload finishes")]
    final_timeout: f64,
    #[arg(long, default_value = "", help = "Write the final JSON result to this path; empty disables file output")]
    output_json: String,
    #[arg(long, default_value_t = false, help = "Reduce console output to only essential lines")]
    quiet: bool,
    #[arg(long, default_value = "", help = "Path to doubao_credentials.json; defaults to the current working directory")]
    credentials: String,
}

fn save_result_json(path: &str, result: &serde_json::Value) -> Result<()> {
    if path.trim().is_empty() {
        return Ok(());
    }
    let target_path = PathBuf::from(path);
    if let Some(parent) = target_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create output directory: {}", parent.display()))?;
        }
    }
    fs::write(&target_path, format!("{}\n", serde_json::to_string_pretty(result)?))
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
        for (index, device) in devices.iter().enumerate() {
            let marker = if device.is_default_input { " (default)" } else { "" };
            println!(
                "[{index}] {}{} | {} Hz / {} ch / {}",
                device.name, marker, device.default_sample_rate, device.channels, device.sample_format
            );
        }
        return Ok(());
    }

    let credentials_path = if args.credentials.trim().is_empty() {
        default_credentials_path()
    } else {
        PathBuf::from(args.credentials.trim())
    };
    let credentials = resolve_credentials(
        Some(&args.app_id),
        Some(&args.access_token),
        &credentials_path,
    )?;

    let mut options = SessionOptions::default();
    options.app_id = credentials.app_id;
    options.access_token = credentials.access_token;
    options.resource_id = args.resource_id.trim().to_owned();
    options.ws_url = args.ws_url.trim().to_owned();
    options.connect_id = (!args.connect_id.trim().is_empty()).then(|| args.connect_id.trim().to_owned());
    options.uid = if args.uid.trim().is_empty() {
        format!("uid-{}", &Uuid::new_v4().simple().to_string()[..12])
    } else {
        args.uid.trim().to_owned()
    };
    options.audio_format = args.audio_format.trim().to_owned();
    options.sample_rate = args.sample_rate;
    options.bits = args.bits;
    options.channels = args.channels;
    options.language = (!args.language.trim().is_empty()).then(|| args.language.trim().to_owned());
    options.chunk_ms = args.chunk_ms;
    options.chunk_bytes = args.chunk_bytes;
    options.send_interval_ms = args.send_interval_ms;
    options.model_name = args.model_name.trim().to_owned();
    options.enable_itn = args.enable_itn;
    options.enable_punc = args.enable_punc;
    options.enable_ddc = args.enable_ddc;
    options.enable_nonstream = args.enable_nonstream;
    options.show_utterances = args.show_utterances;
    options.result_type = args.result_type.trim().to_owned();
    options.timeout_seconds = args.timeout;
    options.final_timeout_seconds = args.final_timeout;

    let result = match args.input_source {
        InputSource::File => {
            if args.audio_file.trim().is_empty() {
                bail!("--audio-file is required when --input-source file");
            }
            let audio_bytes = fs::read(&args.audio_file)
                .with_context(|| format!("Failed to read audio file: {}", args.audio_file))?;
            if audio_bytes.is_empty() {
                bail!("Audio file is empty: {}", args.audio_file);
            }
            if !args.quiet {
                println!("Connecting: {}", options.ws_url);
                println!("Input source: file");
                println!("Audio bytes: {}", audio_bytes.len());
            }
            run_file_session(&options, &audio_bytes, |text, is_final| {
                if args.quiet || text.trim().is_empty() {
                    return;
                }
                if is_final {
                    println!("[ASR FINAL] {text}");
                } else {
                    println!("[ASR] {text}");
                }
            })
            .await?
        }
        InputSource::Mic => {
            let mut mic_options = options.clone();
            mic_options.sample_rate = MIC_UPLOAD_SAMPLE_RATE;
            mic_options.bits = MIC_UPLOAD_BITS;
            mic_options.channels = MIC_UPLOAD_CHANNELS;
            if !args.quiet {
                println!("Connecting: {}", options.ws_url);
                println!("Input source: mic");
                println!(
                    "Microphone capture: duration={:.1}s selector={}",
                    args.mic_duration,
                    if args.mic_device.trim().is_empty() {
                        "<default>"
                    } else {
                        args.mic_device.trim()
                    }
                );
                println!(
                    "Upload format: {} Hz / {} ch / {}-bit PCM (local capture is converted only if needed)",
                    mic_options.sample_rate,
                    mic_options.channels,
                    mic_options.bits
                );
            }
            let capture_options = MicrophoneCaptureOptions {
                duration_seconds: args.mic_duration,
                device_selector: (!args.mic_device.trim().is_empty())
                    .then(|| args.mic_device.trim().to_owned()),
                chunk_ms: args.chunk_ms,
                chunk_bytes: args.chunk_bytes,
                stop_signal: None,
                target_sample_rate: mic_options.sample_rate,
                target_channels: mic_options.channels,
            };
            let result = run_mic_session(&mic_options, &capture_options, |text, is_final| {
                if args.quiet || text.trim().is_empty() {
                    return;
                }
                if is_final {
                    println!("[ASR FINAL] {text}");
                } else {
                    println!("[ASR] {text}");
                }
            })
            .await?;
            if !args.quiet {
                println!(
                    "Capture source: {} Hz / {} ch / {}{}",
                    result.capture_source_sample_rate.unwrap_or_default(),
                    result.capture_source_channels.unwrap_or_default(),
                    result.capture_sample_format.as_deref().unwrap_or("unknown"),
                    if result.capture_direct_target_format == Some(true) {
                        " (direct target format)"
                    } else {
                        " (converted)"
                    }
                );
            }
            result
        }
    };

    let json_result = json!({
        "connect_id": result.connect_id,
        "handshake_headers": result.handshake_headers,
        "final_text": result.final_text,
        "got_final": result.got_final,
        "sent_audio_bytes": result.sent_audio_bytes,
        "sent_chunk_count": result.sent_chunk_count,
        "logs": result.logs,
    });

    let final_text = json_result["final_text"].as_str().unwrap_or_default().trim();
    if result.got_final {
        println!("ASR Final Text: {final_text}");
    } else if final_text.is_empty() {
        println!("ASR did not return a final result before timeout.");
    } else {
        println!("ASR did not return a final result before timeout. Latest interim text: {final_text}");
    }
    save_result_json(&args.output_json, &json_result)?;
    Ok(())
}
