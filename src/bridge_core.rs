use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_json::json;

use crate::asr::{SessionOptions, run_mic_session};
use crate::audio::MicrophoneCaptureOptions;
use crate::platform::{CueKind, PlatformServices, WindowInfo};
use crate::preview::{PreviewMode, PreviewPrinter};

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
}

pub async fn run_bridge_loop(
    config: &BridgeRuntimeConfig,
    session_options: &SessionOptions,
    platform: &mut PlatformServices,
) -> Result<()> {
    let mut preview = PreviewPrinter::new(config.preview_mode);
    let mut utterance_index = 0usize;

    print_startup_banner(config);

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

                let session_result = run_mic_session(session_options, &capture_options, |text, is_final| {
                    preview.show(text, is_final);
                }).await;

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
                            "connect_id": result.connect_id,
                            "handshake_headers": result.handshake_headers,
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

fn print_startup_banner(config: &BridgeRuntimeConfig) {
    let uses_fn_ptt = ptt_key_uses_fn(&config.ptt_key_display);
    let uses_caps_lock_ptt = ptt_key_uses_caps_lock(&config.ptt_key_display);
    println!("[BRIDGE] Rust voice bridge started.");
    println!("[BRIDGE] The foreground window at long-press time becomes the target input window.");
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
