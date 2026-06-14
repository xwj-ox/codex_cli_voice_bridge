use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_json::json;

use crate::asr::{SessionOptions, run_mic_session as run_doubao_mic_session};
use crate::audio::MicrophoneCaptureOptions;
use crate::mai::{MaiOptions, run_mic_session_with_callback as run_mai_mic_session};
use crate::platform::{CueKind, PlatformServices, WindowInfo};
use crate::preview::{PreviewMode, PreviewPrinter};

#[derive(Debug, Clone)]
pub enum BridgeAsrProvider {
    Doubao(SessionOptions),
    Mai(MaiOptions),
}

impl BridgeAsrProvider {
    fn name(&self) -> &'static str {
        match self {
            BridgeAsrProvider::Doubao(_) => "doubao",
            BridgeAsrProvider::Mai(_) => "mai",
        }
    }
}

struct BridgeRecognitionResult {
    provider: &'static str,
    session_id: String,
    response_headers: BTreeMap<String, String>,
    final_text: String,
    got_final: bool,
    sent_audio_bytes: usize,
    sent_chunk_count: usize,
    capture_source_sample_rate: Option<u32>,
    capture_source_channels: Option<u16>,
    capture_sample_format: Option<String>,
    capture_direct_target_format: Option<bool>,
    logs: serde_json::Value,
}

pub struct BridgeRuntimeConfig {
    pub mic_duration: f32,
    pub mic_device: Option<String>,
    pub chunk_ms: u32,
    pub chunk_bytes: usize,
    pub preview_mode: PreviewMode,
    pub submit: bool,
    pub paste_delay_ms: u64,
    pub forbid_host_window_target: bool,
    pub require_title: String,
    pub cue_sounds: bool,
    pub output_json: String,
    pub ptt_key_display: String,
    pub ptt_hold_ms: u64,
    pub ptt_short_press_passthrough: bool,
    pub upload_sample_rate: u32,
    pub upload_channels: u16,
    pub upload_bits: u16,
    pub asr_auto_context: bool,
    pub asr_auto_context_max_items: usize,
    pub asr_auto_context_max_bytes: usize,
}

pub async fn run_bridge_loop(
    config: &BridgeRuntimeConfig,
    asr_provider: &BridgeAsrProvider,
    platform: &mut PlatformServices,
) -> Result<()> {
    let mut preview = PreviewPrinter::new(config.preview_mode);
    let mut utterance_index = 0usize;

    print_startup_banner(config, asr_provider);
    let mut auto_context = AutoDialogContext::new(
        config.asr_auto_context,
        config.asr_auto_context_max_items,
        config.asr_auto_context_max_bytes,
    );

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("\n[BRIDGE] stopped by Ctrl-C");
                break;
            }
            activation = platform.ptt.recv_activation() => {
                let Some(activation) = activation else {
                    break;
                };
                let target_window = activation.target_window;
                let target_capture_missing = target_window.process_name.trim().is_empty();
                let target_is_usable = platform.window_service.is_usable_target(
                    Some(&target_window),
                    &config.require_title,
                    config.forbid_host_window_target,
                );
                if target_is_usable {
                    print_target_window(&target_window);
                } else if target_capture_missing {
                    println!(
                        "[BRIDGE] no foreground paste target was captured; recognition will still run, but this utterance will not be pasted."
                    );
                } else {
                    println!(
                        "[BRIDGE] current foreground window is not an allowed target ({} | {}); skipping this utterance.",
                        target_window.process_name,
                        if target_window.title.is_empty() { "<no title>" } else { target_window.title.as_str() }
                    );
                    play_cue_if_enabled(config, &*platform.cue_player, CueKind::Error);
                    continue;
                }

                utterance_index += 1;
                println!("\n[BRIDGE] listening for utterance #{} ...", utterance_index);
                play_cue_if_enabled(config, &*platform.cue_player, CueKind::Listen);
                preview.reset();

                let capture_options = MicrophoneCaptureOptions {
                    duration_seconds: config.mic_duration,
                    device_selector: config.mic_device.clone(),
                    chunk_ms: config.chunk_ms,
                    chunk_bytes: config.chunk_bytes,
                    stop_signal: Some(activation.stop_signal.clone()),
                    target_sample_rate: config.upload_sample_rate,
                    target_channels: config.upload_channels,
                };

                let session_result = match asr_provider {
                    BridgeAsrProvider::Doubao(session_options) => {
                        let mut utterance_session_options = session_options.clone();
                        if let Some(context) = auto_context.build_context() {
                            utterance_session_options.corpus_context = Some(context);
                        } else if config.asr_auto_context {
                            // Explicitly clear corpus.context so static hotwords do not accidentally stick around.
                            utterance_session_options.corpus_context = None;
                        }

                        run_doubao_mic_session(&utterance_session_options, &capture_options, |text, is_final| {
                            preview.show(text, is_final);
                        })
                        .await
                        .map(|result| BridgeRecognitionResult {
                            provider: "doubao",
                            session_id: result.connect_id,
                            response_headers: result.handshake_headers,
                            final_text: result.final_text,
                            got_final: result.got_final,
                            sent_audio_bytes: result.sent_audio_bytes,
                            sent_chunk_count: result.sent_chunk_count,
                            capture_source_sample_rate: result.capture_source_sample_rate,
                            capture_source_channels: result.capture_source_channels,
                            capture_sample_format: result.capture_sample_format,
                            capture_direct_target_format: result.capture_direct_target_format,
                            logs: json!(result.logs),
                        })
                    }
                    BridgeAsrProvider::Mai(options) => run_mai_mic_session(options, &capture_options, |text, is_final| {
                        preview.show(text, is_final);
                    })
                        .await
                        .map(|result| BridgeRecognitionResult {
                            provider: "mai",
                            session_id: result.request_id.unwrap_or_default(),
                            response_headers: result.response_headers,
                            final_text: result.final_text,
                            got_final: result.got_final,
                            sent_audio_bytes: result.sent_audio_bytes,
                            sent_chunk_count: result.sent_chunk_count,
                            capture_source_sample_rate: result.capture_source_sample_rate,
                            capture_source_channels: result.capture_source_channels,
                            capture_sample_format: result.capture_sample_format,
                            capture_direct_target_format: result.capture_direct_target_format,
                            logs: json!([result.response_json]),
                        }),
                };

                match session_result {
                    Ok(result) => {
                        preview.finish();
                        println!(
                            "[BRIDGE] capture source: {} Hz / {} ch / {}{}",
                            result.capture_source_sample_rate.unwrap_or_default(),
                            result.capture_source_channels.unwrap_or_default(),
                            result.capture_sample_format.as_deref().unwrap_or("unknown"),
                            if result.capture_direct_target_format == Some(true) {
                                " (direct target format)"
                            } else {
                                " (converted)"
                            }
                        );
                        let json_result = json!({
                            "provider": result.provider,
                            "session_id": result.session_id,
                            "connect_id": if result.provider == "doubao" { json!(result.session_id) } else { serde_json::Value::Null },
                            "request_id": if result.provider == "mai" { json!(result.session_id) } else { serde_json::Value::Null },
                            "response_headers": result.response_headers,
                            "final_text": result.final_text,
                            "got_final": result.got_final,
                            "sent_audio_bytes": result.sent_audio_bytes,
                            "sent_chunk_count": result.sent_chunk_count,
                            "capture_source_sample_rate": result.capture_source_sample_rate,
                            "capture_source_channels": result.capture_source_channels,
                            "capture_sample_format": result.capture_sample_format,
                            "capture_direct_target_format": result.capture_direct_target_format,
                            "logs": result.logs,
                        });
                        let _ = save_result_json(&json_result, &config.output_json, utterance_index)?;

                        let final_text = json_result["final_text"].as_str().unwrap_or_default().trim().to_owned();
                        if !result.got_final {
                            if final_text.is_empty() {
                                println!(
                                    "[BRIDGE] ASR did not return a final result before timeout; nothing was pasted. sent_audio_bytes={}, sent_chunk_count={}",
                                    result.sent_audio_bytes,
                                    result.sent_chunk_count
                                );
                            } else {
                                println!(
                                    "[BRIDGE] ASR did not return a final result before timeout; latest interim text was not pasted: {}",
                                    final_text
                                );
                            }
                            play_cue_if_enabled(config, &*platform.cue_player, CueKind::Error);
                            continue;
                        }
                        if final_text.is_empty() {
                            println!(
                                "[BRIDGE] empty final text; nothing pasted. got_final={}, sent_audio_bytes={}, sent_chunk_count={}",
                                result.got_final,
                                result.sent_audio_bytes,
                                result.sent_chunk_count
                            );
                            play_cue_if_enabled(config, &*platform.cue_player, CueKind::Error);
                            continue;
                        }

                        auto_context.ingest_final_text(&final_text);
                        println!("[BRIDGE] final text #{}: {}", utterance_index, final_text);
                        play_cue_if_enabled(config, &*platform.cue_player, CueKind::Recognized);
                        if !target_is_usable {
                            println!("[BRIDGE] no usable paste target captured; final text was not pasted.");
                            play_cue_if_enabled(config, &*platform.cue_player, CueKind::Error);
                            continue;
                        }
                        if let Err(error) = platform.text_injector.paste_text(
                            &target_window,
                            &final_text,
                            config.submit,
                            config.paste_delay_ms,
                        ) {
                            println!("[BRIDGE] paste failed: {error}");
                            play_cue_if_enabled(config, &*platform.cue_player, CueKind::Error);
                            continue;
                        }

                        println!(
                            "[BRIDGE] pasted utterance #{} into the target window{}",
                            utterance_index,
                            if config.submit { " and submitted." } else { "." }
                        );
                        play_cue_if_enabled(config, &*platform.cue_player, CueKind::Pasted);
                    }
                    Err(error) => {
                        preview.finish();
                        println!("[BRIDGE] ASR failed: {error}");
                        play_cue_if_enabled(config, &*platform.cue_player, CueKind::Error);
                    }
                }
            }
        }
    }

    Ok(())
}

fn print_startup_banner(config: &BridgeRuntimeConfig, asr_provider: &BridgeAsrProvider) {
    let uses_fn_ptt = ptt_key_uses_fn(&config.ptt_key_display);
    let uses_caps_lock_ptt = ptt_key_uses_caps_lock(&config.ptt_key_display);
    println!("[BRIDGE] Rust voice bridge started.");
    println!("[BRIDGE] ASR provider: {}", asr_provider.name());
    println!("[BRIDGE] The foreground window at initial key-down becomes the target input window.");
    println!(
        "[BRIDGE] Hold `{}` to talk, release to finish one utterance. {}",
        config.ptt_key_display,
        if config.submit {
            "Auto-submit is ON."
        } else {
            "Text will be pasted only; press Enter yourself to send."
        }
    );
    println!(
        "[BRIDGE] PTT activates after holding for {} ms. Short taps are {}.",
        config.ptt_hold_ms,
        if uses_fn_ptt {
            "not replayed for `fn`"
        } else if config.ptt_short_press_passthrough {
            "passed through normally"
        } else {
            "ignored"
        }
    );
    if uses_fn_ptt {
        println!(
            "[BRIDGE] macOS note: using `fn` as the PTT key disables Fn/Globe shortcuts and `fn+...` key combinations while the bridge is running."
        );
    }
    if uses_caps_lock_ptt && cfg!(target_os = "macos") {
        println!(
            "[BRIDGE] macOS note: `capslock` is a system special key. While the bridge is running, it can still switch between non-Latin and Latin input sources, or trigger Caps Lock / continuous uppercase behavior."
        );
    }
    println!(
        "[BRIDGE] upload format: {} Hz / {} ch / {}-bit PCM (local capture is converted only if needed)",
        config.upload_sample_rate, config.upload_channels, config.upload_bits
    );
    match asr_provider {
        BridgeAsrProvider::Doubao(session_options) => {
            if config.asr_auto_context {
                println!(
                    "[BRIDGE] contextual ASR: auto dialog context is ON (max_items={}, max_bytes={}).",
                    config.asr_auto_context_max_items, config.asr_auto_context_max_bytes
                );
                println!(
                    "[BRIDGE] contextual ASR: recent final texts will be sent as corpus.context for future utterances."
                );
            }
            print_doubao_context_banner(config, session_options);
        }
        BridgeAsrProvider::Mai(options) => {
            println!(
                "[BRIDGE] MAI: transport={}, model={}, locales={}, style={}.",
                options.transport.as_str(),
                options.model,
                if options.locales.is_empty() {
                    "auto".to_owned()
                } else {
                    options.locales.join(",")
                },
                options.style
            );
            if options.transport.as_str() == "voice-live" {
                println!(
                    "[BRIDGE] MAI Voice Live: session_model={}, api_version={}, turn_detection={}.",
                    options.live_model, options.live_api_version, options.live_turn_detection
                );
            }
            if !options.phrases.is_empty() {
                println!(
                    "[BRIDGE] MAI: phrase list is enabled ({} phrases).",
                    options.phrases.len()
                );
            }
            if config.asr_auto_context {
                println!(
                    "[BRIDGE] contextual ASR: auto dialog context is ignored by MAI; use --mai-phrase for entity bias."
                );
            }
        }
    }
    if !config.require_title.trim().is_empty() {
        println!(
            "[BRIDGE] Window title must contain: {}",
            config.require_title.trim()
        );
    }
    if config.forbid_host_window_target {
        println!("[BRIDGE] Host window targeting is disabled for this run.");
    }
    println!("[BRIDGE] Press Ctrl-C here to exit.");
}

fn print_doubao_context_banner(config: &BridgeRuntimeConfig, session_options: &SessionOptions) {
    if session_options
        .corpus_boosting_table_name
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || session_options
            .corpus_boosting_table_id
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        || session_options
            .corpus_correct_table_name
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        || session_options
            .corpus_correct_table_id
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        || session_options
            .corpus_context
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    {
        println!("[BRIDGE] contextual ASR: corpus hints are enabled.");
        if session_options
            .corpus_boosting_table_name
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            println!("[BRIDGE] contextual ASR: boosting_table_name is set.");
        }
        if session_options
            .corpus_boosting_table_id
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            println!("[BRIDGE] contextual ASR: boosting_table_id is set.");
        }
        if session_options
            .corpus_correct_table_name
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            println!("[BRIDGE] contextual ASR: correct_table_name is set.");
        }
        if session_options
            .corpus_correct_table_id
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            println!("[BRIDGE] contextual ASR: correct_table_id is set.");
        }
        if let Some(value) = session_options
            .corpus_context
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            if config.asr_auto_context {
                println!(
                    "[BRIDGE] contextual ASR: note: static corpus.context will be ignored while auto dialog context is enabled."
                );
            }
            println!(
                "[BRIDGE] contextual ASR: corpus.context is set ({} bytes).",
                value.len()
            );
        }
    }
}

fn print_target_window(window: &WindowInfo) {
    let title = if window.title.is_empty() {
        "<no title>"
    } else {
        window.title.as_str()
    };
    println!(
        "[BRIDGE] target window: {} | {}",
        window.process_name, title
    );
}

fn play_cue_if_enabled(
    config: &BridgeRuntimeConfig,
    cue_player: &dyn crate::platform::CuePlayer,
    cue: CueKind,
) {
    if config.cue_sounds {
        cue_player.play(cue);
    }
}

struct AutoDialogContext {
    enabled: bool,
    max_items: usize,
    max_bytes: usize,
    // Stored newest-first to match the API guidance (new -> old).
    items_newest_first: VecDeque<String>,
}

impl AutoDialogContext {
    fn new(enabled: bool, max_items: usize, max_bytes: usize) -> Self {
        Self {
            enabled,
            max_items: max_items.max(1),
            max_bytes,
            items_newest_first: VecDeque::new(),
        }
    }

    fn ingest_final_text(&mut self, text: &str) {
        if !self.enabled {
            return;
        }
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }

        if self
            .items_newest_first
            .front()
            .is_some_and(|latest| latest == trimmed)
        {
            return;
        }

        self.items_newest_first.push_front(trimmed.to_owned());
        while self.items_newest_first.len() > self.max_items {
            self.items_newest_first.pop_back();
        }
    }

    fn build_context(&self) -> Option<String> {
        if !self.enabled || self.items_newest_first.is_empty() {
            return None;
        }

        let mut items: Vec<String> = self.items_newest_first.iter().cloned().collect();
        loop {
            let context = json!({
                "context_type": "dialog_ctx",
                "context_data": items.iter().map(|text| json!({ "text": text })).collect::<Vec<_>>(),
            })
            .to_string();

            if self.max_bytes > 0 && context.len() > self.max_bytes && items.len() > 1 {
                items.pop(); // drop the oldest entry
                continue;
            }

            return Some(context);
        }
    }
}

fn ptt_key_uses_fn(key_name: &str) -> bool {
    matches!(
        key_name.trim().to_ascii_lowercase().as_str(),
        "fn" | "function" | "globe"
    )
}

fn ptt_key_uses_caps_lock(key_name: &str) -> bool {
    matches!(
        key_name.trim().to_ascii_lowercase().as_str(),
        "capslock" | "caps-lock" | "caps_lock"
    )
}

pub fn save_result_json(
    result: &serde_json::Value,
    output_json: &str,
    session_index: usize,
) -> Result<Option<PathBuf>> {
    let output_value = output_json.trim();
    if output_value.is_empty() {
        return Ok(None);
    }

    let output_path = PathBuf::from(output_value);
    let suffix = output_path
        .extension()
        .and_then(|value| value.to_str())
        .map(|ext| format!(".{ext}"))
        .unwrap_or_else(|| ".json".to_owned());
    let target_path = output_path.with_file_name(format!(
        "{}_{session_index:04}{}",
        output_path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("bridge_result"),
        suffix
    ));

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
    println!("Saved result JSON: {}", target_path.display());
    Ok(Some(target_path))
}
