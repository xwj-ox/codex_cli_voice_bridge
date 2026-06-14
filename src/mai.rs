use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use futures_util::{SinkExt, StreamExt};
use http::header::HeaderValue;
use reqwest::StatusCode;
use reqwest::multipart::{Form, Part};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::time::{sleep, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::audio::{CaptureEvent, MicrophoneCaptureOptions, start_microphone_capture};

static TLS_PROVIDER_INIT: OnceLock<Result<(), String>> = OnceLock::new();

pub const DEFAULT_MAI_API_VERSION: &str = "2025-10-15";
pub const DEFAULT_MAI_MODEL: &str = "mai-transcribe-1.5";
pub const DEFAULT_MAI_LIVE_API_VERSION: &str = "2026-04-10";
pub const DEFAULT_MAI_LIVE_MODEL: &str = "gpt-4.1";
pub const DEFAULT_MAI_LIVE_TURN_DETECTION: &str = "none";
pub const DEFAULT_MAI_LIVE_SILENCE_DURATION_MS: u32 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaiTransport {
    Rest,
    VoiceLive,
}

impl MaiTransport {
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "rest" | "sync" | "synchronous" => Ok(Self::Rest),
            "voice-live" | "voicelive" | "live" | "websocket" | "ws" => Ok(Self::VoiceLive),
            other => bail!("Unsupported MAI transport: {other}. Use rest or voice-live"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rest => "rest",
            Self::VoiceLive => "voice-live",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MaiOptions {
    pub transport: MaiTransport,
    pub endpoint: String,
    pub key: String,
    pub api_version: String,
    pub model: String,
    pub locales: Vec<String>,
    pub style: String,
    pub phrases: Vec<String>,
    pub timeout_seconds: f64,
    pub max_retries: usize,
    pub live_api_version: String,
    pub live_model: String,
    pub live_turn_detection: String,
    pub live_silence_duration_ms: u32,
}

impl Default for MaiOptions {
    fn default() -> Self {
        Self {
            transport: MaiTransport::Rest,
            endpoint: String::new(),
            key: String::new(),
            api_version: DEFAULT_MAI_API_VERSION.to_owned(),
            model: DEFAULT_MAI_MODEL.to_owned(),
            locales: Vec::new(),
            style: "default".to_owned(),
            phrases: Vec::new(),
            timeout_seconds: 600.0,
            max_retries: 4,
            live_api_version: DEFAULT_MAI_LIVE_API_VERSION.to_owned(),
            live_model: DEFAULT_MAI_LIVE_MODEL.to_owned(),
            live_turn_detection: DEFAULT_MAI_LIVE_TURN_DETECTION.to_owned(),
            live_silence_duration_ms: DEFAULT_MAI_LIVE_SILENCE_DURATION_MS,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MaiResult {
    pub request_id: Option<String>,
    pub response_headers: BTreeMap<String, String>,
    pub final_text: String,
    pub got_final: bool,
    pub sent_audio_bytes: usize,
    pub sent_chunk_count: usize,
    pub capture_source_sample_rate: Option<u32>,
    pub capture_source_channels: Option<u16>,
    pub capture_sample_format: Option<String>,
    pub capture_direct_target_format: Option<bool>,
    pub response_json: Value,
}

impl MaiOptions {
    pub fn validate(&self) -> Result<()> {
        if self.endpoint.trim().is_empty() {
            bail!("Missing required MAI endpoint");
        }
        if self.key.trim().is_empty() {
            bail!("Missing required MAI key");
        }
        if !matches!(self.model.trim(), "mai-transcribe-1" | "mai-transcribe-1.5") {
            bail!(
                "Unsupported MAI model: {}. Use mai-transcribe-1 or mai-transcribe-1.5",
                self.model
            );
        }
        if self.transport == MaiTransport::VoiceLive {
            if self.model.trim() != "mai-transcribe-1" {
                bail!("MAI Voice Live transport currently supports only mai-transcribe-1");
            }
            if self.live_api_version.trim().is_empty() {
                bail!("Missing required MAI Voice Live API version");
            }
            if self.live_model.trim().is_empty() {
                bail!("Missing required MAI Voice Live session model");
            }
            validate_live_turn_detection(&self.live_turn_detection)?;
        }
        if !matches!(self.style.trim(), "default" | "verbatim") {
            bail!("Unsupported MAI style: {}", self.style);
        }
        if self.model.trim() != "mai-transcribe-1.5" && self.style.trim() == "verbatim" {
            bail!("MAI style 'verbatim' is only supported by mai-transcribe-1.5");
        }
        if self.model.trim() != "mai-transcribe-1.5" && !self.phrases.is_empty() {
            bail!("MAI phrase list is only supported by mai-transcribe-1.5");
        }
        if self.phrases.len() > 200 {
            bail!("MAI 1.5 supports at most 200 phrases");
        }
        Ok(())
    }

    pub fn definition(&self) -> Result<Value> {
        self.validate()?;

        let locales = self
            .locales
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();

        let mut enhanced_mode = json!({
            "enabled": true,
            "task": "transcribe",
            "model": self.model.trim(),
        });
        if self.model.trim() == "mai-transcribe-1.5" && self.style.trim() != "default" {
            enhanced_mode["transcribeStyle"] = json!(self.style.trim());
        }

        let mut definition = json!({
            "enhancedMode": enhanced_mode,
        });
        if !locales.is_empty() {
            definition["locales"] = json!(locales);
        }
        if self.model.trim() == "mai-transcribe-1.5" && !self.phrases.is_empty() {
            let phrases = self
                .phrases
                .iter()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();
            if !phrases.is_empty() {
                definition["phraseList"] = json!({ "phrases": phrases });
            }
        }
        Ok(definition)
    }
}

pub async fn run_file_session(
    options: &MaiOptions,
    audio_bytes: &[u8],
    file_name: &str,
    content_type: &str,
) -> Result<MaiResult> {
    if options.transport == MaiTransport::VoiceLive {
        bail!("MAI Voice Live transport only supports microphone streaming");
    }
    run_file_session_with_capture_metadata(
        options,
        audio_bytes,
        file_name,
        content_type,
        None,
        None,
        None,
        None,
        1,
    )
    .await
}

pub async fn run_mic_session(
    options: &MaiOptions,
    capture_options: &MicrophoneCaptureOptions,
) -> Result<MaiResult> {
    run_mic_session_with_callback(options, capture_options, |_, _| {}).await
}

pub async fn run_mic_session_with_callback<F>(
    options: &MaiOptions,
    capture_options: &MicrophoneCaptureOptions,
    on_text: F,
) -> Result<MaiResult>
where
    F: FnMut(&str, bool),
{
    match options.transport {
        MaiTransport::Rest => run_rest_mic_session(options, capture_options).await,
        MaiTransport::VoiceLive => {
            run_voice_live_mic_session(options, capture_options, on_text).await
        }
    }
}

async fn run_rest_mic_session(
    options: &MaiOptions,
    capture_options: &MicrophoneCaptureOptions,
) -> Result<MaiResult> {
    options.validate()?;
    let mut capture = start_microphone_capture(capture_options)?;
    let source_sample_rate = capture.source_sample_rate;
    let source_channels = capture.source_channels;
    let sample_rate = capture.sample_rate;
    let channels = capture.channels;
    let sample_format = capture.sample_format.clone();
    let direct_target_format = capture.direct_target_format;

    let mut pcm_bytes = Vec::new();
    let mut sent_chunk_count = 0usize;
    while let Some(event) = capture.recv().await {
        match event {
            CaptureEvent::Chunk { data, is_last } => {
                sent_chunk_count += 1;
                pcm_bytes.extend_from_slice(&data);
                if is_last {
                    break;
                }
            }
            CaptureEvent::Error(message) => bail!(message),
        }
    }

    if pcm_bytes.is_empty() {
        bail!("No microphone audio captured. Check device permissions and settings.");
    }

    let wav_bytes = pcm16_to_wav(&pcm_bytes, sample_rate, 16, channels);
    run_file_session_with_capture_metadata(
        options,
        &wav_bytes,
        "microphone.wav",
        "audio/wav",
        Some(source_sample_rate),
        Some(source_channels),
        Some(sample_format),
        Some(direct_target_format),
        sent_chunk_count,
    )
    .await
}

async fn run_voice_live_mic_session<F>(
    options: &MaiOptions,
    capture_options: &MicrophoneCaptureOptions,
    mut on_text: F,
) -> Result<MaiResult>
where
    F: FnMut(&str, bool),
{
    options.validate()?;
    ensure_tls_provider()?;

    let mut capture = start_microphone_capture(capture_options)?;
    let source_sample_rate = capture.source_sample_rate;
    let source_channels = capture.source_channels;
    let sample_rate = capture.sample_rate;
    let channels = capture.channels;
    let sample_format = capture.sample_format.clone();
    let direct_target_format = capture.direct_target_format;

    if channels != 1 {
        bail!("MAI Voice Live microphone mode currently only supports mono PCM16");
    }

    let url = build_voice_live_url(options)?;
    let mut request = url
        .into_client_request()
        .context("Failed to create MAI Voice Live WebSocket request")?;
    request.headers_mut().insert(
        "api-key",
        HeaderValue::from_str(options.key.trim()).context("Invalid MAI key header value")?,
    );

    let (mut ws, response) = connect_async(request)
        .await
        .context("Failed to connect to MAI Voice Live WebSocket")?;
    let response_headers = response
        .headers()
        .iter()
        .map(|(key, value)| {
            (
                key.to_string(),
                value.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect::<BTreeMap<_, _>>();

    let session_update = build_voice_live_session_update(options, sample_rate)?;
    ws.send(Message::Text(session_update.to_string().into()))
        .await
        .context("Failed to send MAI Voice Live session.update")?;

    let mut state = VoiceLiveState::default();
    wait_for_voice_live_session_update(&mut ws, &mut state).await?;

    let mut sent_audio_bytes = 0usize;
    let mut sent_chunk_count = 0usize;

    while !state.got_final {
        let Some(event) = capture.recv().await else {
            break;
        };
        match event {
            CaptureEvent::Chunk { data, is_last } => {
                if !data.is_empty() {
                    let event = json!({
                        "type": "input_audio_buffer.append",
                        "audio": BASE64_STANDARD.encode(&data),
                    });
                    ws.send(Message::Text(event.to_string().into()))
                        .await
                        .context("Failed to send MAI Voice Live audio chunk")?;
                    sent_audio_bytes += data.len();
                    sent_chunk_count += 1;
                }
                drain_voice_live_events(
                    &mut ws,
                    &mut state,
                    Duration::from_millis(1),
                    &mut on_text,
                )
                .await?;
                if is_last {
                    break;
                }
            }
            CaptureEvent::Error(message) => bail!(message),
        }
    }

    if !state.got_final && !state.committed {
        ws.send(Message::Text(
            json!({ "type": "input_audio_buffer.commit" })
                .to_string()
                .into(),
        ))
        .await
        .context("Failed to send MAI Voice Live input_audio_buffer.commit")?;
        state.committed = true;
    }

    let deadline = Duration::from_secs_f64(options.timeout_seconds.max(1.0));
    let started = tokio::time::Instant::now();
    while !state.got_final && started.elapsed() < deadline {
        let remaining = deadline.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            break;
        }
        let wait = remaining.min(Duration::from_millis(500));
        if !recv_one_voice_live_event(&mut ws, &mut state, wait, &mut on_text).await? {
            continue;
        }
    }

    let _ = ws.close(None).await;

    Ok(MaiResult {
        request_id: state.item_id.or(state.session_id),
        response_headers,
        final_text: state.final_text.trim().to_owned(),
        got_final: state.got_final,
        sent_audio_bytes,
        sent_chunk_count,
        capture_source_sample_rate: Some(source_sample_rate),
        capture_source_channels: Some(source_channels),
        capture_sample_format: Some(sample_format),
        capture_direct_target_format: Some(direct_target_format),
        response_json: Value::Array(state.logs),
    })
}

async fn run_file_session_with_capture_metadata(
    options: &MaiOptions,
    audio_bytes: &[u8],
    file_name: &str,
    content_type: &str,
    capture_source_sample_rate: Option<u32>,
    capture_source_channels: Option<u16>,
    capture_sample_format: Option<String>,
    capture_direct_target_format: Option<bool>,
    sent_chunk_count: usize,
) -> Result<MaiResult> {
    options.validate()?;
    if audio_bytes.is_empty() {
        bail!("Audio bytes are empty");
    }

    let definition = options.definition()?;
    let url = build_transcribe_url(options);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs_f64(options.timeout_seconds.max(1.0)))
        .build()
        .context("Failed to build HTTP client")?;

    let mut last_error: Option<anyhow::Error> = None;
    for attempt in 0..=options.max_retries {
        let form = build_multipart_form(audio_bytes, file_name, content_type, &definition)?;
        let response = client
            .post(&url)
            .header("Ocp-Apim-Subscription-Key", options.key.trim())
            .multipart(form)
            .send()
            .await;

        match response {
            Ok(response) if response.status().is_success() => {
                let headers = response
                    .headers()
                    .iter()
                    .map(|(key, value)| {
                        (
                            key.to_string(),
                            value.to_str().unwrap_or_default().to_owned(),
                        )
                    })
                    .collect::<BTreeMap<_, _>>();
                let request_id = headers
                    .get("apim-request-id")
                    .or_else(|| headers.get("x-ms-request-id"))
                    .cloned();
                let response_json = response
                    .json::<Value>()
                    .await
                    .context("Failed to parse MAI JSON response")?;
                let final_text = extract_transcript(&response_json);
                let got_final = !final_text.trim().is_empty();
                return Ok(MaiResult {
                    request_id,
                    response_headers: headers,
                    final_text,
                    got_final,
                    sent_audio_bytes: audio_bytes.len(),
                    sent_chunk_count,
                    capture_source_sample_rate,
                    capture_source_channels,
                    capture_sample_format,
                    capture_direct_target_format,
                    response_json,
                });
            }
            Ok(response) => {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                let error = anyhow!("MAI request failed: HTTP {status}\n{body}");
                if !is_retriable(status) || attempt >= options.max_retries {
                    return Err(error);
                }
                last_error = Some(error);
            }
            Err(error) => {
                let error = anyhow!("MAI request failed: {error}");
                if attempt >= options.max_retries {
                    return Err(error);
                }
                last_error = Some(error);
            }
        }

        let delay_seconds = 2u64.pow(attempt.min(5) as u32);
        sleep(Duration::from_secs(delay_seconds)).await;
    }

    Err(last_error.unwrap_or_else(|| anyhow!("MAI request failed after retries")))
}

fn build_transcribe_url(options: &MaiOptions) -> String {
    format!(
        "{}/speechtotext/transcriptions:transcribe?api-version={}",
        options.endpoint.trim().trim_end_matches('/'),
        options.api_version.trim()
    )
}

#[derive(Debug, Default)]
struct VoiceLiveState {
    session_id: Option<String>,
    item_id: Option<String>,
    final_text: String,
    got_final: bool,
    committed: bool,
    logs: Vec<Value>,
}

fn validate_live_turn_detection(value: &str) -> Result<()> {
    match value.trim().to_ascii_lowercase().as_str() {
        ""
        | "none"
        | "off"
        | "server_vad"
        | "azure_semantic_vad"
        | "azure_semantic_vad_multilingual" => Ok(()),
        other => bail!(
            "Unsupported MAI Voice Live turn detection: {other}. Use none, server_vad, azure_semantic_vad, or azure_semantic_vad_multilingual"
        ),
    }
}

fn build_voice_live_url(options: &MaiOptions) -> Result<String> {
    let endpoint = options.endpoint.trim().trim_end_matches('/');
    if endpoint.is_empty() {
        bail!("Missing required MAI endpoint");
    }
    let websocket_endpoint = if let Some(rest) = endpoint.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = endpoint.strip_prefix("http://") {
        format!("ws://{rest}")
    } else if endpoint.starts_with("wss://") || endpoint.starts_with("ws://") {
        endpoint.to_owned()
    } else {
        format!("wss://{endpoint}")
    };

    Ok(format!(
        "{}/voice-live/realtime?api-version={}&model={}",
        websocket_endpoint,
        percent_encode_query_component(options.live_api_version.trim()),
        percent_encode_query_component(options.live_model.trim())
    ))
}

fn build_voice_live_session_update(options: &MaiOptions, sample_rate: u32) -> Result<Value> {
    let language = options
        .locales
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(",");

    let mut transcription = json!({
        "model": "mai-transcribe-1",
    });
    if !language.is_empty() {
        transcription["language"] = json!(language);
    }

    let turn_detection = match options
        .live_turn_detection
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "" | "none" | "off" => Value::Null,
        value => json!({
            "type": value,
            "silence_duration_ms": options.live_silence_duration_ms,
            "create_response": false,
        }),
    };

    Ok(json!({
        "type": "session.update",
        "session": {
            "modalities": ["text"],
            "input_audio_format": "pcm16",
            "input_audio_sampling_rate": sample_rate,
            "input_audio_transcription": transcription,
            "turn_detection": turn_detection,
        }
    }))
}

async fn wait_for_voice_live_session_update<S>(
    ws: &mut tokio_tungstenite::WebSocketStream<S>,
    state: &mut VoiceLiveState,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let deadline = Duration::from_secs(10);
    let started = tokio::time::Instant::now();
    while started.elapsed() < deadline {
        let remaining = deadline.saturating_sub(started.elapsed());
        if !recv_one_voice_live_event(
            ws,
            state,
            remaining.min(Duration::from_millis(500)),
            &mut |_, _| {},
        )
        .await?
        {
            continue;
        }
        if state.session_id.is_some() {
            return Ok(());
        }
    }
    bail!("Timed out waiting for MAI Voice Live session.updated");
}

async fn drain_voice_live_events<S, F>(
    ws: &mut tokio_tungstenite::WebSocketStream<S>,
    state: &mut VoiceLiveState,
    wait: Duration,
    on_text: &mut F,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    F: FnMut(&str, bool),
{
    while recv_one_voice_live_event(ws, state, wait, on_text).await? {
        if state.got_final {
            break;
        }
    }
    Ok(())
}

async fn recv_one_voice_live_event<S, F>(
    ws: &mut tokio_tungstenite::WebSocketStream<S>,
    state: &mut VoiceLiveState,
    wait: Duration,
    on_text: &mut F,
) -> Result<bool>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    F: FnMut(&str, bool),
{
    let message = match timeout(wait, ws.next()).await {
        Err(_) => return Ok(false),
        Ok(None) => return Ok(false),
        Ok(Some(Ok(message))) => message,
        Ok(Some(Err(error))) => {
            return Err(error).context("MAI Voice Live WebSocket receive failed");
        }
    };

    match message {
        Message::Text(text) => {
            let value = serde_json::from_str::<Value>(&text)
                .with_context(|| format!("Failed to parse MAI Voice Live JSON event: {text}"))?;
            process_voice_live_event(value, state, on_text)?;
            Ok(true)
        }
        Message::Binary(data) => {
            let value = serde_json::from_slice::<Value>(&data)
                .context("Failed to parse MAI Voice Live binary JSON event")?;
            process_voice_live_event(value, state, on_text)?;
            Ok(true)
        }
        Message::Close(_) => Ok(false),
        _ => Ok(true),
    }
}

fn process_voice_live_event<F>(
    value: Value,
    state: &mut VoiceLiveState,
    on_text: &mut F,
) -> Result<()>
where
    F: FnMut(&str, bool),
{
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();

    match event_type.as_str() {
        "session.updated" => {
            state.session_id = value
                .pointer("/session/id")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        "input_audio_buffer.committed" => {
            state.committed = true;
            state.item_id = value
                .get("item_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        "conversation.item.input_audio_transcription.delta" => {
            if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                state.final_text.push_str(delta);
                on_text(&state.final_text, false);
            }
        }
        "conversation.item.input_audio_transcription.completed" => {
            state.final_text = extract_voice_live_transcript(&value);
            state.got_final = !state.final_text.trim().is_empty();
            on_text(&state.final_text, true);
        }
        "conversation.item.input_audio_transcription.failed" => {
            return Err(anyhow!(
                "MAI Voice Live transcription failed: {}",
                format_voice_live_error(value.get("error"))
            ));
        }
        "error" => {
            return Err(anyhow!(
                "MAI Voice Live error: {}",
                format_voice_live_error(value.get("error"))
            ));
        }
        _ => {}
    }

    state.logs.push(value);
    Ok(())
}

fn extract_voice_live_transcript(value: &Value) -> String {
    if let Some(text) = value.get("transcript").and_then(Value::as_str) {
        return text.trim().to_owned();
    }
    value
        .get("phrases")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn format_voice_live_error(error: Option<&Value>) -> String {
    let Some(error) = error else {
        return "<missing error payload>".to_owned();
    };
    if let Some(message) = error.get("message").and_then(Value::as_str) {
        let code = error
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let param = error.get("param").and_then(Value::as_str).unwrap_or("");
        if param.is_empty() {
            return format!("{code}: {message}");
        }
        return format!("{code}: {message} (param={param})");
    }
    error.to_string()
}

fn ensure_tls_provider() -> Result<()> {
    let init_result = TLS_PROVIDER_INIT.get_or_init(|| {
        rustls::crypto::ring::default_provider()
            .install_default()
            .map_err(|error| format!("{error:?}"))
    });

    match init_result {
        Ok(()) => Ok(()),
        Err(error) => bail!("Failed to initialize rustls CryptoProvider: {error}"),
    }
}

fn percent_encode_query_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn build_multipart_form(
    audio_bytes: &[u8],
    file_name: &str,
    content_type: &str,
    definition: &Value,
) -> Result<Form> {
    let audio = Part::bytes(audio_bytes.to_vec())
        .file_name(file_name.to_owned())
        .mime_str(content_type)
        .with_context(|| format!("Invalid audio MIME type: {content_type}"))?;
    Ok(Form::new()
        .part("audio", audio)
        .text("definition", definition.to_string()))
}

fn is_retriable(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::TOO_MANY_REQUESTS
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

pub fn guess_audio_content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "mp4" => "audio/mp4",
        "flac" => "audio/flac",
        "ogg" => "audio/ogg",
        "oga" => "audio/ogg",
        "webm" => "audio/webm",
        _ => "application/octet-stream",
    }
}

pub fn extract_transcript(response_json: &Value) -> String {
    if let Some(text) = response_json
        .get("combinedPhrases")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|text| !text.trim().is_empty())
    {
        return text;
    }

    if let Some(text) = response_json
        .get("phrases")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|text| !text.trim().is_empty())
    {
        return text;
    }

    response_json
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

pub fn pcm16_to_wav(pcm_bytes: &[u8], sample_rate: u32, bits: u16, channels: u16) -> Vec<u8> {
    let data_len = pcm_bytes.len() as u32;
    let byte_rate = sample_rate * channels as u32 * bits as u32 / 8;
    let block_align = channels * bits / 8;
    let riff_len = 36u32.saturating_add(data_len);
    let mut wav = Vec::with_capacity(44 + pcm_bytes.len());

    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff_len.to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&bits.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(pcm_bytes);
    wav
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mai_transport_parse_accepts_voice_live_aliases() {
        assert_eq!(
            MaiTransport::parse("voice-live").unwrap(),
            MaiTransport::VoiceLive
        );
        assert_eq!(MaiTransport::parse("ws").unwrap(), MaiTransport::VoiceLive);
        assert_eq!(MaiTransport::parse("rest").unwrap(), MaiTransport::Rest);
        assert!(MaiTransport::parse("bogus").is_err());
    }

    #[test]
    fn voice_live_url_uses_websocket_endpoint_and_model_query() {
        let options = MaiOptions {
            transport: MaiTransport::VoiceLive,
            endpoint: "https://example.cognitiveservices.azure.com/".to_owned(),
            key: "key".to_owned(),
            model: "mai-transcribe-1".to_owned(),
            live_model: "gpt-4.1 custom".to_owned(),
            ..MaiOptions::default()
        };

        assert_eq!(
            build_voice_live_url(&options).unwrap(),
            "wss://example.cognitiveservices.azure.com/voice-live/realtime?api-version=2026-04-10&model=gpt-4.1%20custom"
        );
    }

    #[test]
    fn voice_live_session_update_uses_transcription_model_and_ptt_commit_by_default() {
        let options = MaiOptions {
            transport: MaiTransport::VoiceLive,
            endpoint: "https://example.cognitiveservices.azure.com".to_owned(),
            key: "key".to_owned(),
            model: "mai-transcribe-1".to_owned(),
            locales: vec!["zh".to_owned(), "en".to_owned()],
            ..MaiOptions::default()
        };

        let update = build_voice_live_session_update(&options, 16000).unwrap();
        assert_eq!(update["type"], "session.update");
        assert_eq!(
            update["session"]["input_audio_transcription"]["model"],
            "mai-transcribe-1"
        );
        assert_eq!(
            update["session"]["input_audio_transcription"]["language"],
            "zh,en"
        );
        assert_eq!(update["session"]["turn_detection"], Value::Null);
    }

    #[test]
    fn rest_definition_omits_locales_by_default() {
        let options = MaiOptions {
            endpoint: "https://example.cognitiveservices.azure.com".to_owned(),
            key: "key".to_owned(),
            ..MaiOptions::default()
        };

        let definition = options.definition().unwrap();
        assert!(definition.get("locales").is_none());
        assert_eq!(
            definition["enhancedMode"]["model"],
            MaiOptions::default().model
        );
    }

    #[test]
    fn voice_live_rejects_mai_transcribe_1_5() {
        let options = MaiOptions {
            transport: MaiTransport::VoiceLive,
            endpoint: "https://example.cognitiveservices.azure.com".to_owned(),
            key: "key".to_owned(),
            model: "mai-transcribe-1.5".to_owned(),
            ..MaiOptions::default()
        };

        let error = options.validate().unwrap_err().to_string();
        assert!(error.contains("supports only mai-transcribe-1"));
    }
}
