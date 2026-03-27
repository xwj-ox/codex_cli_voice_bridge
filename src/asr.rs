use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use futures_util::{SinkExt, StreamExt};
use http::header::HeaderValue;
use serde::Serialize;
use serde_json::Value;
use tokio::time::{sleep, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use crate::audio::{CaptureEvent, MicrophoneCaptureOptions, start_microphone_capture};
use crate::protocol::{
    DEFAULT_RESOURCE_ID, DEFAULT_WS_URL, MSG_TYPE_ERROR_RESPONSE, ParsedFrame,
    RequestAudioOptions, RequestRuntimeOptions, build_audio_request, build_full_client_request,
    build_full_payload, extract_text, frame_to_json_value, is_final_frame, parse_response_frame,
};

static TLS_PROVIDER_INIT: OnceLock<Result<(), String>> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct SessionOptions {
    pub app_id: String,
    pub access_token: String,
    pub resource_id: String,
    pub ws_url: String,
    pub connect_id: Option<String>,
    pub uid: String,
    pub audio_format: String,
    pub sample_rate: u32,
    pub bits: u16,
    pub channels: u16,
    pub language: Option<String>,
    pub chunk_ms: u32,
    pub chunk_bytes: usize,
    pub send_interval_ms: u64,
    pub model_name: String,
    pub enable_itn: bool,
    pub enable_punc: bool,
    pub enable_ddc: bool,
    pub enable_nonstream: bool,
    pub show_utterances: bool,
    pub result_type: String,
    pub timeout_seconds: f64,
    pub final_timeout_seconds: f64,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            app_id: String::new(),
            access_token: String::new(),
            resource_id: DEFAULT_RESOURCE_ID.to_owned(),
            ws_url: DEFAULT_WS_URL.to_owned(),
            connect_id: None,
            uid: format!("uid-{}", &Uuid::new_v4().simple().to_string()[..12]),
            audio_format: "pcm".to_owned(),
            sample_rate: 16000,
            bits: 16,
            channels: 1,
            language: None,
            chunk_ms: 200,
            chunk_bytes: 0,
            send_interval_ms: 200,
            model_name: "bigmodel".to_owned(),
            enable_itn: true,
            enable_punc: true,
            enable_ddc: false,
            enable_nonstream: false,
            show_utterances: true,
            result_type: "full".to_owned(),
            timeout_seconds: 10.0,
            final_timeout_seconds: 15.0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionResult {
    pub connect_id: String,
    pub handshake_headers: BTreeMap<String, String>,
    pub final_text: String,
    pub got_final: bool,
    pub sent_audio_bytes: usize,
    pub sent_chunk_count: usize,
    pub capture_source_sample_rate: Option<u32>,
    pub capture_source_channels: Option<u16>,
    pub capture_sample_format: Option<String>,
    pub capture_direct_target_format: Option<bool>,
    pub logs: Vec<Value>,
}

pub fn infer_chunk_bytes(
    chunk_bytes: usize,
    chunk_ms: u32,
    sample_rate: u32,
    bits: u16,
    channels: u16,
) -> usize {
    if chunk_bytes > 0 {
        return chunk_bytes;
    }
    let bytes_per_sample = (bits as usize).div_ceil(8);
    let bytes_per_second = sample_rate as usize * bytes_per_sample * channels as usize;
    let inferred = (bytes_per_second * chunk_ms as usize) / 1000;
    inferred.max(bytes_per_sample * channels as usize)
}

fn chunk_data(data: &[u8], chunk_size: usize) -> Vec<&[u8]> {
    if chunk_size == 0 {
        return vec![data];
    }
    data.chunks(chunk_size).collect()
}

async fn recv_one_frame<S>(
    ws: &mut tokio_tungstenite::WebSocketStream<S>,
    timeout_duration: Duration,
) -> Result<Option<ParsedFrame>>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    match timeout(timeout_duration, ws.next()).await {
        Err(_) => Ok(None),
        Ok(None) => Ok(None),
        Ok(Some(Ok(Message::Binary(data)))) => parse_response_frame(data.as_ref()).map(Some),
        Ok(Some(Ok(Message::Close(_)))) => Ok(None),
        Ok(Some(Ok(_))) => Ok(None),
        Ok(Some(Err(error))) => Err(error).context("WebSocket receive failed"),
    }
}

fn validate_options(options: &SessionOptions) -> Result<()> {
    if options.app_id.trim().is_empty() {
        bail!("Missing required value: app_id");
    }
    if options.access_token.trim().is_empty() {
        bail!("Missing required value: access_token");
    }
    if options.resource_id.trim().is_empty() {
        bail!("Missing required value: resource_id");
    }
    if options.ws_url.trim().is_empty() {
        bail!("Missing required value: ws_url");
    }
    Ok(())
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

fn build_websocket_request(options: &SessionOptions, connect_id: &str) -> Result<http::Request<()>> {
    let mut request = options
        .ws_url
        .as_str()
        .into_client_request()
        .context("Failed to create WebSocket handshake request")?;

    request.headers_mut().insert(
        "X-Api-App-Key",
        HeaderValue::from_str(options.app_id.trim()).context("Invalid app_id header value")?,
    );
    request.headers_mut().insert(
        "X-Api-Access-Key",
        HeaderValue::from_str(options.access_token.trim())
            .context("Invalid access_token header value")?,
    );
    request.headers_mut().insert(
        "X-Api-Resource-Id",
        HeaderValue::from_str(options.resource_id.trim())
            .context("Invalid resource_id header value")?,
    );
    request.headers_mut().insert(
        "X-Api-Connect-Id",
        HeaderValue::from_str(connect_id).context("Invalid connect_id header value")?,
    );

    Ok(request)
}

pub async fn run_file_session<F>(
    options: &SessionOptions,
    audio_bytes: &[u8],
    mut on_text: F,
) -> Result<SessionResult>
where
    F: FnMut(&str, bool),
{
    validate_options(options)?;
    ensure_tls_provider()?;
    if audio_bytes.is_empty() {
        bail!("Audio bytes are empty");
    }

    let connect_id = options
        .connect_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let request = build_websocket_request(options, &connect_id)?;

    let (mut ws, response) = connect_async(request)
        .await
        .context("Failed to connect to Doubao ASR WebSocket")?;

    let handshake_headers = response
        .headers()
        .iter()
        .map(|(key, value)| {
            (
                key.to_string(),
                value.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect::<BTreeMap<_, _>>();

    let audio = RequestAudioOptions {
        audio_format: options.audio_format.clone(),
        sample_rate: options.sample_rate,
        bits: options.bits,
        channels: options.channels,
        language: options.language.clone(),
    };
    let runtime = RequestRuntimeOptions {
        uid: options.uid.clone(),
        model_name: options.model_name.clone(),
        enable_itn: options.enable_itn,
        enable_punc: options.enable_punc,
        enable_ddc: options.enable_ddc,
        enable_nonstream: options.enable_nonstream,
        show_utterances: options.show_utterances,
        result_type: options.result_type.clone(),
    };
    let payload = build_full_payload(&audio, &runtime);
    ws.send(Message::Binary(build_full_client_request(&payload)?.into()))
        .await
        .context("Failed to send full client request")?;

    let mut logs = Vec::new();
    let mut final_text = String::new();
    let mut got_final = false;
    let mut sent_audio_bytes = 0usize;
    let mut sent_chunk_count = 0usize;
    let chunk_size = infer_chunk_bytes(
        options.chunk_bytes,
        options.chunk_ms,
        options.sample_rate,
        options.bits,
        options.channels,
    );
    let chunks = chunk_data(audio_bytes, chunk_size);

    loop {
        let Some(frame) = recv_one_frame(&mut ws, Duration::from_millis(200)).await? else {
            break;
        };
        logs.push(frame_to_json_value(&frame));
        if frame.message_type == MSG_TYPE_ERROR_RESPONSE {
            bail!(
                "Server error code={:?}, payload={:?}",
                frame.error_code,
                frame.payload
            );
        }
        let text = extract_text(&frame.payload);
        if !text.is_empty() {
            final_text = text.clone();
            on_text(&text, is_final_frame(&frame));
        }
        if is_final_frame(&frame) {
            got_final = true;
            break;
        }
    }

    for (index, chunk) in chunks.iter().enumerate() {
        if got_final {
            break;
        }

        let is_last = index + 1 == chunks.len();
        ws.send(Message::Binary(build_audio_request(chunk, is_last)?.into()))
            .await
            .context("Failed to send audio frame")?;
        sent_audio_bytes += chunk.len();
        sent_chunk_count += 1;

        loop {
            let Some(frame) = recv_one_frame(&mut ws, Duration::from_millis(50)).await? else {
                break;
            };
            logs.push(frame_to_json_value(&frame));
            if frame.message_type == MSG_TYPE_ERROR_RESPONSE {
                bail!(
                    "Server error code={:?}, payload={:?}",
                    frame.error_code,
                    frame.payload
                );
            }
            let text = extract_text(&frame.payload);
            if !text.is_empty() {
                final_text = text.clone();
                on_text(&text, is_final_frame(&frame));
            }
            if is_final_frame(&frame) {
                got_final = true;
                break;
            }
        }

        if got_final {
            break;
        }
        if options.send_interval_ms > 0 {
            sleep(Duration::from_millis(options.send_interval_ms)).await;
        }
    }

    let deadline = Duration::from_secs_f64(options.final_timeout_seconds.max(1.0));
    let started = tokio::time::Instant::now();
    while !got_final && started.elapsed() < deadline {
        let Some(frame) = recv_one_frame(&mut ws, Duration::from_millis(500)).await? else {
            continue;
        };
        logs.push(frame_to_json_value(&frame));
        if frame.message_type == MSG_TYPE_ERROR_RESPONSE {
            bail!(
                "Server error code={:?}, payload={:?}",
                frame.error_code,
                frame.payload
            );
        }
        let text = extract_text(&frame.payload);
        if !text.is_empty() {
            final_text = text.clone();
            on_text(&text, is_final_frame(&frame));
        }
        if is_final_frame(&frame) {
            got_final = true;
        }
    }

    ws.close(None)
        .await
        .map_err(|error| anyhow!("Failed to close WebSocket: {error}"))?;

    Ok(SessionResult {
        connect_id,
        handshake_headers,
        final_text,
        got_final,
        sent_audio_bytes,
        sent_chunk_count,
        capture_source_sample_rate: None,
        capture_source_channels: None,
        capture_sample_format: None,
        capture_direct_target_format: None,
        logs,
    })
}

pub async fn run_mic_session<F>(
    options: &SessionOptions,
    capture_options: &MicrophoneCaptureOptions,
    mut on_text: F,
) -> Result<SessionResult>
where
    F: FnMut(&str, bool),
{
    validate_options(options)?;
    ensure_tls_provider()?;
    if !options.audio_format.trim().eq_ignore_ascii_case("pcm") {
        bail!("Microphone mode currently only supports --audio-format pcm");
    }
    if options.bits != 16 {
        bail!("Microphone mode currently only supports --bits 16");
    }
    if options.channels != 1 {
        bail!("Microphone mode currently only supports mono upload");
    }
    if capture_options.target_sample_rate != options.sample_rate
        || capture_options.target_channels != options.channels
    {
        bail!("Microphone capture target format must match the upload format");
    }

    let mut capture = start_microphone_capture(capture_options)?;
    let mut mic_options = options.clone();
    mic_options.sample_rate = capture.sample_rate;
    mic_options.channels = capture.channels;
    mic_options.bits = 16;

    let connect_id = mic_options
        .connect_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let request = build_websocket_request(&mic_options, &connect_id)?;

    let (mut ws, response) = connect_async(request)
        .await
        .context("Failed to connect to Doubao ASR WebSocket")?;

    let handshake_headers = response
        .headers()
        .iter()
        .map(|(key, value)| {
            (
                key.to_string(),
                value.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect::<BTreeMap<_, _>>();

    let audio = RequestAudioOptions {
        audio_format: mic_options.audio_format.clone(),
        sample_rate: mic_options.sample_rate,
        bits: mic_options.bits,
        channels: mic_options.channels,
        language: mic_options.language.clone(),
    };
    let runtime = RequestRuntimeOptions {
        uid: mic_options.uid.clone(),
        model_name: mic_options.model_name.clone(),
        enable_itn: mic_options.enable_itn,
        enable_punc: mic_options.enable_punc,
        enable_ddc: mic_options.enable_ddc,
        enable_nonstream: mic_options.enable_nonstream,
        show_utterances: mic_options.show_utterances,
        result_type: mic_options.result_type.clone(),
    };
    let payload = build_full_payload(&audio, &runtime);
    ws.send(Message::Binary(build_full_client_request(&payload)?.into()))
        .await
        .context("Failed to send full client request")?;

    let mut logs = Vec::new();
    let mut final_text = String::new();
    let mut got_final = false;
    let mut sent_audio_bytes = 0usize;
    let mut sent_chunk_count = 0usize;

    loop {
        let Some(frame) = recv_one_frame(&mut ws, Duration::from_millis(200)).await? else {
            break;
        };
        logs.push(frame_to_json_value(&frame));
        if frame.message_type == MSG_TYPE_ERROR_RESPONSE {
            bail!(
                "Server error code={:?}, payload={:?}",
                frame.error_code,
                frame.payload
            );
        }
        let text = extract_text(&frame.payload);
        if !text.is_empty() {
            final_text = text.clone();
            on_text(&text, is_final_frame(&frame));
        }
        if is_final_frame(&frame) {
            got_final = true;
            break;
        }
    }

    while !got_final {
        let Some(event) = capture.recv().await else {
            break;
        };
        match event {
            CaptureEvent::Chunk { data, is_last } => {
                ws.send(Message::Binary(build_audio_request(&data, is_last)?.into()))
                    .await
                    .context("Failed to send microphone audio frame")?;
                sent_audio_bytes += data.len();
                sent_chunk_count += 1;

                loop {
                    let Some(frame) = recv_one_frame(&mut ws, Duration::from_millis(50)).await? else {
                        break;
                    };
                    logs.push(frame_to_json_value(&frame));
                    if frame.message_type == MSG_TYPE_ERROR_RESPONSE {
                        bail!(
                            "Server error code={:?}, payload={:?}",
                            frame.error_code,
                            frame.payload
                        );
                    }
                    let text = extract_text(&frame.payload);
                    if !text.is_empty() {
                        final_text = text.clone();
                        on_text(&text, is_final_frame(&frame));
                    }
                    if is_final_frame(&frame) {
                        got_final = true;
                        break;
                    }
                }

                if is_last {
                    break;
                }
            }
            CaptureEvent::Error(message) => bail!(message),
        }
    }

    let deadline = Duration::from_secs_f64(mic_options.final_timeout_seconds.max(1.0));
    let started = tokio::time::Instant::now();
    while !got_final && started.elapsed() < deadline {
        let Some(frame) = recv_one_frame(&mut ws, Duration::from_millis(500)).await? else {
            continue;
        };
        logs.push(frame_to_json_value(&frame));
        if frame.message_type == MSG_TYPE_ERROR_RESPONSE {
            bail!(
                "Server error code={:?}, payload={:?}",
                frame.error_code,
                frame.payload
            );
        }
        let text = extract_text(&frame.payload);
        if !text.is_empty() {
            final_text = text.clone();
            on_text(&text, is_final_frame(&frame));
        }
        if is_final_frame(&frame) {
            got_final = true;
        }
    }

    ws.close(None)
        .await
        .map_err(|error| anyhow!("Failed to close WebSocket: {error}"))?;

    Ok(SessionResult {
        connect_id,
        handshake_headers,
        final_text,
        got_final,
        sent_audio_bytes,
        sent_chunk_count,
        capture_source_sample_rate: Some(capture.source_sample_rate),
        capture_source_channels: Some(capture.source_channels),
        capture_sample_format: Some(capture.sample_format.clone()),
        capture_direct_target_format: Some(capture.direct_target_format),
        logs,
    })
}
