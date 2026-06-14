use anyhow::Result;
use clap::{Parser, ValueEnum};
use serde_json::json;
use std::path::PathBuf;
use uuid::Uuid;

use codex_cli_voice_bridge_rust::asr::SessionOptions;
use codex_cli_voice_bridge_rust::audio::list_input_devices;
use codex_cli_voice_bridge_rust::bridge_core::{
    BridgeAsrProvider, BridgeRuntimeConfig, run_bridge_loop,
};
use codex_cli_voice_bridge_rust::config::{
    default_credentials_path, default_mai_credentials_path, resolve_credentials,
    resolve_mai_credentials,
};
use codex_cli_voice_bridge_rust::mai::{
    DEFAULT_MAI_API_VERSION, DEFAULT_MAI_LIVE_API_VERSION, DEFAULT_MAI_LIVE_MODEL,
    DEFAULT_MAI_LIVE_SILENCE_DURATION_MS, DEFAULT_MAI_LIVE_TURN_DETECTION, DEFAULT_MAI_MODEL,
    MaiOptions, MaiTransport,
};
use codex_cli_voice_bridge_rust::platform::{PlatformInitOptions, create_platform_services};
use codex_cli_voice_bridge_rust::preview::PreviewMode;
use codex_cli_voice_bridge_rust::protocol::{DEFAULT_RESOURCE_ID, DEFAULT_WS_URL};

const MIC_UPLOAD_SAMPLE_RATE: u32 = 16000;
const MIC_UPLOAD_BITS: u16 = 16;
const MIC_UPLOAD_CHANNELS: u16 = 1;
const DEFAULT_PTT_KEY: &str = "capslock";
#[cfg(target_os = "macos")]
const PTT_KEY_HELP: &str = "PTT key: space, enter, capslock, fn, left-win, right-control, right-shift, f1-f12, or a single letter";
#[cfg(not(target_os = "macos"))]
const PTT_KEY_HELP: &str = "PTT key: space, enter, capslock, left-win, right-control, right-shift, f1-f12, or a single letter";

#[derive(Debug, Clone, Copy, ValueEnum)]
enum AsrProvider {
    Doubao,
    Mai,
}

#[derive(Debug, Parser)]
#[command(
    name = "voice_bridge",
    about = "Push-to-talk ASR bridge",
    long_about = "Capture one utterance with a push-to-talk key, send it to the selected ASR provider, and paste the final text back into the foreground window.",
    next_line_help = true
)]
struct Args {
    #[arg(long, value_enum, default_value_t = AsrProvider::Doubao, help = "ASR provider: doubao or mai")]
    asr_provider: AsrProvider,
    #[arg(
        long,
        default_value = "",
        help = "Doubao app_id; falls back to the credentials file when omitted"
    )]
    app_id: String,
    #[arg(
        long,
        default_value = "",
        help = "Doubao access_token; falls back to the credentials file when omitted"
    )]
    access_token: String,
    #[arg(long, default_value = DEFAULT_RESOURCE_ID, help = "Doubao ASR resource_id; docs list volc.bigasr.sauc.duration/concurrent for 1.0 and volc.seedasr.sauc.duration/concurrent for 2.0")]
    resource_id: String,
    #[arg(long, default_value = DEFAULT_WS_URL, help = "Doubao WebSocket URL; default is the optimized bidirectional endpoint /api/v3/sauc/bigmodel_async")]
    ws_url: String,
    #[arg(
        long,
        default_value = "",
        help = "Path to doubao_credentials.json; defaults to the current working directory"
    )]
    credentials: String,
    #[arg(long, default_value = "", help = "Azure Speech endpoint for MAI")]
    mai_endpoint: String,
    #[arg(long, default_value = "", help = "Azure Speech key for MAI")]
    mai_key: String,
    #[arg(
        long,
        default_value = "",
        help = "Path to mai_credentials.json; defaults to the current working directory"
    )]
    mai_credentials: String,
    #[arg(long, default_value = DEFAULT_MAI_API_VERSION, help = "Azure Speech API version for MAI")]
    mai_api_version: String,
    #[arg(
        long,
        default_value = "rest",
        help = "MAI transport: rest or voice-live"
    )]
    mai_transport: String,
    #[arg(long, default_value = DEFAULT_MAI_MODEL, help = "MAI model: mai-transcribe-1 or mai-transcribe-1.5")]
    mai_model: String,
    #[arg(
        long,
        value_delimiter = ',',
        help = "Optional MAI locale hint; repeat or comma-separate values"
    )]
    mai_locale: Vec<String>,
    #[arg(
        long,
        default_value = "default",
        help = "MAI style: default or verbatim"
    )]
    mai_style: String,
    #[arg(
        long,
        help = "MAI 1.5 phrase list entry; repeat to add multiple phrases"
    )]
    mai_phrase: Vec<String>,
    #[arg(
        long,
        default_value_t = 600.0,
        help = "MAI request/final-result timeout in seconds"
    )]
    mai_timeout: f64,
    #[arg(
        long,
        default_value_t = 4,
        help = "MAI retry count for 429 and transient 5xx responses"
    )]
    mai_max_retries: usize,
    #[arg(
        long,
        default_value = DEFAULT_MAI_LIVE_API_VERSION,
        help = "MAI Voice Live API version"
    )]
    mai_live_api_version: String,
    #[arg(
        long,
        default_value = DEFAULT_MAI_LIVE_MODEL,
        help = "MAI Voice Live session model, for example gpt-4.1"
    )]
    mai_live_model: String,
    #[arg(
        long,
        default_value = DEFAULT_MAI_LIVE_TURN_DETECTION,
        help = "MAI Voice Live turn detection: none, server_vad, azure_semantic_vad, or azure_semantic_vad_multilingual"
    )]
    mai_live_turn_detection: String,
    #[arg(
        long,
        default_value_t = DEFAULT_MAI_LIVE_SILENCE_DURATION_MS,
        help = "MAI Voice Live silence duration in milliseconds when turn detection is enabled"
    )]
    mai_live_silence_duration_ms: u32,
    #[arg(
        long,
        default_value_t = 16000,
        help = "Reserved compatibility option; upload sample rate is fixed to 16000 Hz"
    )]
    sample_rate: u32,
    #[arg(
        long,
        default_value_t = 16,
        help = "Reserved compatibility option; upload bit depth is fixed to 16-bit PCM"
    )]
    bits: u16,
    #[arg(
        long,
        default_value_t = 1,
        help = "Reserved compatibility option; upload channel count is fixed to mono"
    )]
    channels: u16,
    #[arg(
        long,
        default_value = "",
        help = "Optional language hint; official docs say it is only supported by bigmodel_nostream and not by the second-pass recognition path"
    )]
    language: String,
    #[arg(
        long,
        default_value_t = 60.0,
        help = "Maximum duration in seconds for one push-to-talk recording"
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
        help = "Audio chunk size before upload; official docs recommend about 100-200 ms, and 200 ms is preferred for bigmodel_async"
    )]
    chunk_ms: u32,
    #[arg(
        long,
        default_value_t = 0,
        help = "Optional fixed chunk size in bytes; 0 means derive from chunk_ms"
    )]
    chunk_bytes: usize,
    #[arg(long, default_value = "bigmodel", help = "Doubao ASR model name")]
    model_name: String,
    #[arg(
        long,
        default_value_t = true,
        help = "Enable ITN text normalization, e.g. turning spoken numerals into written forms like year 1970 or amount $123"
    )]
    enable_itn: bool,
    #[arg(
        long,
        default_value_t = true,
        help = "Enable punctuation insertion; official docs say this defaults to true"
    )]
    enable_punc: bool,
    #[arg(
        long,
        default_value_t = false,
        help = "Enable semantic smoothing (DDC, likely Disfluency Detection and Correction), which removes fillers, hesitations, and repeated words to improve readability; official docs say this defaults to false"
    )]
    enable_ddc: bool,
    #[arg(
        long,
        default_value_t = true,
        help = "Enable second-pass recognition: on the optimized bidirectional API, each finalized VAD segment is re-recognized with the nostream model to improve final accuracy"
    )]
    enable_nonstream: bool,
    #[arg(
        long,
        default_value_t = true,
        help = "Return utterance-level segmentation details such as pauses, sentence splits, and word information"
    )]
    show_utterances: bool,
    #[arg(
        long,
        default_value = "full",
        help = "Result return mode: full returns all utterances each time; single returns only the current utterance and is intended to be used with show_utterances=true"
    )]
    result_type: String,
    #[arg(
        long,
        help = "Contextual ASR hotword; repeat to add multiple words (sets request.corpus.context as a hotwords JSON string)"
    )]
    asr_hotword: Vec<String>,
    #[arg(
        long,
        default_value = "",
        help = "Raw request.corpus.context string (a JSON string payload); when set, --asr-hotword is ignored"
    )]
    asr_corpus_context: String,
    #[arg(
        long,
        default_value = "",
        help = "Doubao self-learning hotword table name (request.corpus.boosting_table_name)"
    )]
    asr_boosting_table_name: String,
    #[arg(
        long,
        default_value = "",
        help = "Doubao self-learning hotword table id (request.corpus.boosting_table_id)"
    )]
    asr_boosting_table_id: String,
    #[arg(
        long,
        default_value = "",
        help = "Doubao self-learning replacement table name (request.corpus.correct_table_name)"
    )]
    asr_correct_table_name: String,
    #[arg(
        long,
        default_value = "",
        help = "Doubao self-learning replacement table id (request.corpus.correct_table_id)"
    )]
    asr_correct_table_id: String,
    #[arg(
        long,
        default_value_t = false,
        help = "Enable automatic dialog context across utterances (request.corpus.context dialog_ctx)"
    )]
    asr_auto_context: bool,
    #[arg(
        long,
        default_value_t = 8,
        help = "Max dialog context items to keep (newest first; API supports up to 20 rounds)"
    )]
    asr_auto_context_max_items: usize,
    #[arg(
        long,
        default_value_t = 4096,
        help = "Max UTF-8 bytes for the generated corpus.context JSON string (older items are dropped first)"
    )]
    asr_auto_context_max_bytes: usize,
    #[arg(
        long,
        default_value_t = 10.0,
        help = "Handshake and per-frame timeout in seconds"
    )]
    timeout: f64,
    #[arg(
        long,
        default_value_t = 15.0,
        help = "Final result wait timeout in seconds after audio upload finishes"
    )]
    final_timeout: f64,
    #[arg(
        long,
        default_value = DEFAULT_PTT_KEY,
        help = PTT_KEY_HELP
    )]
    ptt_key: String,
    #[arg(
        long,
        default_value_t = 250,
        help = "Hold duration in milliseconds before the PTT key starts recording"
    )]
    ptt_hold_ms: u64,
    #[arg(
        long,
        default_value_t = true,
        help = "Replay a short tap of the PTT key as a normal key press"
    )]
    ptt_short_press_passthrough: bool,
    #[arg(long, value_enum, default_value_t = PreviewMode::Single, help = "Preview mode for interim recognition text")]
    preview_mode: PreviewMode,
    #[arg(
        long,
        default_value_t = false,
        help = "Press Enter after pasting the final text"
    )]
    submit: bool,
    #[arg(
        long,
        default_value_t = 90,
        help = "Delay in milliseconds before pasting after refocusing the target window"
    )]
    paste_delay_ms: u64,
    #[arg(
        long,
        default_value_t = false,
        help = "Disallow using the bridge host window as the paste target"
    )]
    forbid_host_window_target: bool,
    #[arg(
        long,
        default_value = "",
        help = "Only allow target windows whose title contains this substring"
    )]
    require_title: String,
    #[arg(
        long,
        default_value_t = true,
        help = "Play cue beeps for listen / recognized / error states"
    )]
    cue_sounds: bool,
    #[arg(
        long,
        default_value = "",
        help = "Write per-utterance JSON results to this base path; empty disables file output"
    )]
    output_json: String,
}

fn build_session_options(args: &Args, app_id: String, access_token: String) -> SessionOptions {
    let mut options = SessionOptions::default();
    options.app_id = app_id;
    options.access_token = access_token;
    options.resource_id = args.resource_id.trim().to_owned();
    options.ws_url = args.ws_url.trim().to_owned();
    options.connect_id = None;
    options.uid = format!("voice-{}", &Uuid::new_v4().simple().to_string()[..12]);
    options.audio_format = "pcm".to_owned();
    options.sample_rate = MIC_UPLOAD_SAMPLE_RATE;
    options.bits = MIC_UPLOAD_BITS;
    options.channels = MIC_UPLOAD_CHANNELS;
    options.language = (!args.language.trim().is_empty()).then(|| args.language.trim().to_owned());
    options.chunk_ms = args.chunk_ms;
    options.chunk_bytes = args.chunk_bytes;
    options.send_interval_ms = 0;
    options.model_name = args.model_name.trim().to_owned();
    options.enable_itn = args.enable_itn;
    options.enable_punc = args.enable_punc;
    options.enable_ddc = args.enable_ddc;
    options.enable_nonstream = args.enable_nonstream;
    options.show_utterances = args.show_utterances;
    options.result_type = args.result_type.trim().to_owned();
    options.corpus_boosting_table_name = (!args.asr_boosting_table_name.trim().is_empty())
        .then(|| args.asr_boosting_table_name.trim().to_owned());
    options.corpus_boosting_table_id = (!args.asr_boosting_table_id.trim().is_empty())
        .then(|| args.asr_boosting_table_id.trim().to_owned());
    options.corpus_correct_table_name = (!args.asr_correct_table_name.trim().is_empty())
        .then(|| args.asr_correct_table_name.trim().to_owned());
    options.corpus_correct_table_id = (!args.asr_correct_table_id.trim().is_empty())
        .then(|| args.asr_correct_table_id.trim().to_owned());
    if !args.asr_corpus_context.trim().is_empty() {
        options.corpus_context = Some(args.asr_corpus_context.trim().to_owned());
    } else {
        let hotwords = args
            .asr_hotword
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if !hotwords.is_empty() {
            options.corpus_context = Some(
                json!({
                    "hotwords": hotwords.into_iter().map(|word| json!({ "word": word })).collect::<Vec<_>>(),
                })
                .to_string(),
            );
        }
    }
    options.timeout_seconds = args.timeout;
    options.final_timeout_seconds = args.final_timeout;
    options
}

fn build_mai_options(args: &Args, endpoint: String, key: String) -> Result<MaiOptions> {
    Ok(MaiOptions {
        transport: MaiTransport::parse(&args.mai_transport)?,
        endpoint,
        key,
        api_version: args.mai_api_version.trim().to_owned(),
        model: args.mai_model.trim().to_owned(),
        locales: args.mai_locale.clone(),
        style: args.mai_style.trim().to_owned(),
        phrases: args
            .mai_phrase
            .iter()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .collect(),
        timeout_seconds: args.mai_timeout,
        max_retries: args.mai_max_retries,
        live_api_version: args.mai_live_api_version.trim().to_owned(),
        live_model: args.mai_live_model.trim().to_owned(),
        live_turn_detection: args.mai_live_turn_detection.trim().to_owned(),
        live_silence_duration_ms: args.mai_live_silence_duration_ms,
    })
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

    let asr_provider = match args.asr_provider {
        AsrProvider::Doubao => {
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
            BridgeAsrProvider::Doubao(build_session_options(
                &args,
                credentials.app_id.clone(),
                credentials.access_token.clone(),
            ))
        }
        AsrProvider::Mai => {
            let credentials_path = if args.mai_credentials.trim().is_empty() {
                default_mai_credentials_path()
            } else {
                PathBuf::from(args.mai_credentials.trim())
            };
            let credentials = resolve_mai_credentials(
                Some(&args.mai_endpoint),
                Some(&args.mai_key),
                &credentials_path,
            )?;
            BridgeAsrProvider::Mai(build_mai_options(
                &args,
                credentials.endpoint,
                credentials.key,
            )?)
        }
    };
    let mut platform = create_platform_services(&PlatformInitOptions {
        ptt_key: args.ptt_key.clone(),
        ptt_hold_ms: args.ptt_hold_ms,
        ptt_short_press_passthrough: args.ptt_short_press_passthrough,
        forbid_host_window_target: args.forbid_host_window_target,
    })?;
    let runtime = BridgeRuntimeConfig {
        mic_duration: args.mic_duration,
        mic_device: (!args.mic_device.trim().is_empty()).then(|| args.mic_device.trim().to_owned()),
        chunk_ms: args.chunk_ms,
        chunk_bytes: args.chunk_bytes,
        preview_mode: args.preview_mode,
        submit: args.submit,
        paste_delay_ms: args.paste_delay_ms,
        forbid_host_window_target: args.forbid_host_window_target,
        require_title: args.require_title.clone(),
        cue_sounds: args.cue_sounds,
        output_json: args.output_json.clone(),
        ptt_key_display: args.ptt_key.clone(),
        ptt_hold_ms: args.ptt_hold_ms,
        ptt_short_press_passthrough: args.ptt_short_press_passthrough,
        upload_sample_rate: MIC_UPLOAD_SAMPLE_RATE,
        upload_channels: MIC_UPLOAD_CHANNELS,
        upload_bits: MIC_UPLOAD_BITS,
        asr_auto_context: args.asr_auto_context,
        asr_auto_context_max_items: args.asr_auto_context_max_items,
        asr_auto_context_max_bytes: args.asr_auto_context_max_bytes,
    };

    run_bridge_loop(&runtime, &asr_provider, &mut platform).await
}
