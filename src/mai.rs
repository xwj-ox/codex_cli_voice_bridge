use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use reqwest::StatusCode;
use reqwest::multipart::{Form, Part};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::time::sleep;

use crate::audio::{CaptureEvent, MicrophoneCaptureOptions, start_microphone_capture};

pub const DEFAULT_MAI_API_VERSION: &str = "2025-10-15";
pub const DEFAULT_MAI_MODEL: &str = "mai-transcribe-1.5";

#[derive(Debug, Clone)]
pub struct MaiOptions {
    pub endpoint: String,
    pub key: String,
    pub api_version: String,
    pub model: String,
    pub locales: Vec<String>,
    pub style: String,
    pub phrases: Vec<String>,
    pub timeout_seconds: f64,
    pub max_retries: usize,
}

impl Default for MaiOptions {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            key: String::new(),
            api_version: DEFAULT_MAI_API_VERSION.to_owned(),
            model: DEFAULT_MAI_MODEL.to_owned(),
            locales: vec!["zh".to_owned()],
            style: "default".to_owned(),
            phrases: Vec::new(),
            timeout_seconds: 600.0,
            max_retries: 4,
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
        if self
            .locales
            .iter()
            .map(|value| value.trim())
            .all(str::is_empty)
        {
            bail!("At least one MAI locale is required");
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
            "locales": locales,
            "enhancedMode": enhanced_mode,
        });
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
