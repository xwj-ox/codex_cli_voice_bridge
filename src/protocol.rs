use std::io::{Read, Write};

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::Serialize;
use serde_json::{Map, Value, json};

pub const DEFAULT_WS_URL: &str = "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async";
pub const DEFAULT_RESOURCE_ID: &str = "volc.seedasr.sauc.duration";

pub const PROTOCOL_VERSION: u8 = 0x1;
pub const HEADER_SIZE_UNITS: u8 = 0x1;

pub const MSG_TYPE_FULL_CLIENT_REQUEST: u8 = 0x1;
pub const MSG_TYPE_AUDIO_ONLY_REQUEST: u8 = 0x2;
pub const MSG_TYPE_FULL_SERVER_RESPONSE: u8 = 0x9;
pub const MSG_TYPE_ERROR_RESPONSE: u8 = 0xF;

pub const FLAG_NONE: u8 = 0x0;
pub const FLAG_LAST_PACKET: u8 = 0x2;
pub const FLAG_POS_SEQUENCE: u8 = 0x1;
pub const FLAG_NEG_SEQUENCE: u8 = 0x3;

pub const SERIALIZATION_NONE: u8 = 0x0;
pub const SERIALIZATION_JSON: u8 = 0x1;

pub const COMPRESSION_NONE: u8 = 0x0;
pub const COMPRESSION_GZIP: u8 = 0x1;

#[derive(Debug, Clone)]
pub struct RequestAudioOptions {
    pub audio_format: String,
    pub sample_rate: u32,
    pub bits: u16,
    pub channels: u16,
    pub language: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RequestRuntimeOptions {
    pub uid: String,
    pub model_name: String,
    pub enable_itn: bool,
    pub enable_punc: bool,
    pub enable_ddc: bool,
    pub enable_nonstream: bool,
    pub show_utterances: bool,
    pub result_type: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ParsedPayload {
    Json(Value),
    Binary(Vec<u8>),
}

#[derive(Debug, Clone, Serialize)]
pub struct ParsedFrame {
    pub message_type: u8,
    pub flags: u8,
    pub serialization: u8,
    pub compression: u8,
    pub sequence: Option<i32>,
    pub error_code: Option<u32>,
    pub payload_size: u32,
    pub payload: ParsedPayload,
}

pub fn build_header(message_type: u8, flags: u8, serialization: u8, compression: u8) -> [u8; 4] {
    let byte0 = ((PROTOCOL_VERSION & 0x0F) << 4) | (HEADER_SIZE_UNITS & 0x0F);
    let byte1 = ((message_type & 0x0F) << 4) | (flags & 0x0F);
    let byte2 = ((serialization & 0x0F) << 4) | (compression & 0x0F);
    [byte0, byte1, byte2, 0]
}

pub fn gzip_compress(data: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}

fn gzip_decompress(data: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = GzDecoder::new(data);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}

pub fn build_full_payload(audio: &RequestAudioOptions, request: &RequestRuntimeOptions) -> Value {
    let mut audio_obj = Map::new();
    audio_obj.insert("format".to_owned(), json!(audio.audio_format));
    audio_obj.insert("rate".to_owned(), json!(audio.sample_rate));
    audio_obj.insert("bits".to_owned(), json!(audio.bits));
    audio_obj.insert("channel".to_owned(), json!(audio.channels));
    if let Some(language) = audio.language.as_ref().filter(|value| !value.trim().is_empty()) {
        audio_obj.insert("language".to_owned(), json!(language));
    }

    json!({
        "user": { "uid": request.uid },
        "audio": Value::Object(audio_obj),
        "request": {
            "model_name": if request.model_name.trim().is_empty() { "bigmodel" } else { &request.model_name },
            "enable_itn": request.enable_itn,
            "enable_punc": request.enable_punc,
            "enable_ddc": request.enable_ddc,
            "enable_nonstream": request.enable_nonstream,
            "show_utterances": request.show_utterances,
            "result_type": request.result_type
        }
    })
}

pub fn build_full_client_request(payload: &Value) -> Result<Vec<u8>> {
    let payload_bytes = serde_json::to_vec(payload)?;
    let payload_gz = gzip_compress(&payload_bytes)?;
    let mut out = Vec::with_capacity(8 + payload_gz.len());
    out.extend_from_slice(&build_header(
        MSG_TYPE_FULL_CLIENT_REQUEST,
        FLAG_NONE,
        SERIALIZATION_JSON,
        COMPRESSION_GZIP,
    ));
    out.extend_from_slice(&(payload_gz.len() as u32).to_be_bytes());
    out.extend_from_slice(&payload_gz);
    Ok(out)
}

pub fn build_audio_request(audio_chunk: &[u8], is_last: bool) -> Result<Vec<u8>> {
    let payload_gz = gzip_compress(audio_chunk)?;
    let mut out = Vec::with_capacity(8 + payload_gz.len());
    out.extend_from_slice(&build_header(
        MSG_TYPE_AUDIO_ONLY_REQUEST,
        if is_last { FLAG_LAST_PACKET } else { FLAG_NONE },
        SERIALIZATION_NONE,
        COMPRESSION_GZIP,
    ));
    out.extend_from_slice(&(payload_gz.len() as u32).to_be_bytes());
    out.extend_from_slice(&payload_gz);
    Ok(out)
}

pub fn parse_response_frame(frame: &[u8]) -> Result<ParsedFrame> {
    if frame.len() < 8 {
        bail!("Invalid frame length: {}", frame.len());
    }

    let byte0 = frame[0];
    let byte1 = frame[1];
    let byte2 = frame[2];

    let version = (byte0 >> 4) & 0x0F;
    if version != PROTOCOL_VERSION {
        bail!("Unsupported protocol version: {}", version);
    }

    let header_size_units = byte0 & 0x0F;
    let header_size = (header_size_units as usize) * 4;
    if header_size < 4 || frame.len() < header_size + 4 {
        bail!("Invalid header size");
    }

    let message_type = (byte1 >> 4) & 0x0F;
    let flags = byte1 & 0x0F;
    let serialization = (byte2 >> 4) & 0x0F;
    let compression = byte2 & 0x0F;

    let mut offset = header_size;
    let mut sequence = None;
    let mut error_code = None;

    if message_type == MSG_TYPE_FULL_SERVER_RESPONSE
        && (flags == FLAG_POS_SEQUENCE || flags == FLAG_NEG_SEQUENCE)
    {
        if frame.len() < offset + 4 {
            bail!("Missing sequence field");
        }
        sequence = Some(i32::from_be_bytes(frame[offset..offset + 4].try_into()?));
        offset += 4;
    } else if message_type == MSG_TYPE_ERROR_RESPONSE {
        if frame.len() < offset + 4 {
            bail!("Missing error code field");
        }
        error_code = Some(u32::from_be_bytes(frame[offset..offset + 4].try_into()?));
        offset += 4;
    }

    if frame.len() < offset + 4 {
        bail!("Missing payload size field");
    }
    let payload_size = u32::from_be_bytes(frame[offset..offset + 4].try_into()?);
    offset += 4;

    if frame.len() < offset + payload_size as usize {
        bail!("Payload size mismatch");
    }
    let payload_raw = &frame[offset..offset + payload_size as usize];
    let payload_data = match compression {
        COMPRESSION_GZIP => gzip_decompress(payload_raw)?,
        COMPRESSION_NONE => payload_raw.to_vec(),
        other => bail!("Unsupported payload compression: {}", other),
    };

    let payload = match serialization {
        SERIALIZATION_JSON => {
            let value: Value = serde_json::from_slice(&payload_data)
                .context("Failed to decode JSON payload from response frame")?;
            ParsedPayload::Json(value)
        }
        SERIALIZATION_NONE => ParsedPayload::Binary(payload_data),
        other => bail!("Unsupported serialization: {}", other),
    };

    Ok(ParsedFrame {
        message_type,
        flags,
        serialization,
        compression,
        sequence,
        error_code,
        payload_size,
        payload,
    })
}

pub fn extract_text(payload: &ParsedPayload) -> String {
    let ParsedPayload::Json(value) = payload else {
        return String::new();
    };

    if let Some(text) = value
        .get("result")
        .and_then(Value::as_object)
        .and_then(|result| result.get("text"))
        .and_then(Value::as_str)
    {
        return text.trim().to_owned();
    }

    if let Some(chunks) = value.get("result").and_then(Value::as_array) {
        let joined = chunks
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("");
        if !joined.is_empty() {
            return joined;
        }
    }

    value
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_owned()
}

pub fn is_final_frame(frame: &ParsedFrame) -> bool {
    frame.message_type == MSG_TYPE_FULL_SERVER_RESPONSE
        && (frame.flags == FLAG_LAST_PACKET || frame.flags == FLAG_NEG_SEQUENCE)
}

pub fn frame_to_json_value(frame: &ParsedFrame) -> Value {
    let payload = match &frame.payload {
        ParsedPayload::Json(value) => value.clone(),
        ParsedPayload::Binary(bytes) => json!({
            "encoding": "base64",
            "data": BASE64_STANDARD.encode(bytes),
        }),
    };

    json!({
        "message_type": frame.message_type,
        "flags": frame.flags,
        "serialization": frame.serialization,
        "compression": frame.compression,
        "sequence": frame.sequence,
        "error_code": frame.error_code,
        "payload_size": frame.payload_size,
        "payload": payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_header_encodes_expected_nibbles() {
        let header = build_header(
            MSG_TYPE_AUDIO_ONLY_REQUEST,
            FLAG_LAST_PACKET,
            SERIALIZATION_NONE,
            COMPRESSION_GZIP,
        );
        assert_eq!(header[0], 0x11);
        assert_eq!(header[1], 0x22);
        assert_eq!(header[2], 0x01);
        assert_eq!(header[3], 0x00);
    }

    #[test]
    fn parse_response_frame_decodes_gzip_json_payload() {
        let payload = json!({
            "result": { "text": "你好，世界" }
        });
        let payload_bytes = serde_json::to_vec(&payload).unwrap();
        let payload_gz = gzip_compress(&payload_bytes).unwrap();

        let mut frame = Vec::new();
        frame.extend_from_slice(&build_header(
            MSG_TYPE_FULL_SERVER_RESPONSE,
            FLAG_LAST_PACKET,
            SERIALIZATION_JSON,
            COMPRESSION_GZIP,
        ));
        frame.extend_from_slice(&(payload_gz.len() as u32).to_be_bytes());
        frame.extend_from_slice(&payload_gz);

        let parsed = parse_response_frame(&frame).unwrap();
        assert!(is_final_frame(&parsed));
        assert_eq!(extract_text(&parsed.payload), "你好，世界");
    }
}
