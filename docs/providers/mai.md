# MAI Provider

MAI can use two Azure Speech paths in this repository:

- `rest`: Azure Speech LLM Speech synchronous transcription.
- `voice-live`: Azure Voice Live WebSocket input transcription.

User-facing provider names use `mai`; Azure model IDs still use the fixed names `mai-transcribe-1` and `mai-transcribe-1.5`.

## Service

REST transcription:

- Endpoint path: `/speechtotext/transcriptions:transcribe`
- Auth header: `Ocp-Apim-Subscription-Key`
- Default API version: `2025-10-15`
- Supported model IDs: `mai-transcribe-1`, `mai-transcribe-1.5`

Voice Live transcription:

- Endpoint path: `/voice-live/realtime`
- Auth header: `api-key`
- Default API version: `2026-04-10`
- Supported MAI transcription model ID: `mai-transcribe-1`
- Default Voice Live session model: `gpt-4.1`

The default `rest` transport is synchronous. The bridge records one utterance, wraps microphone PCM as WAV, sends the file to Azure, and pastes the final text when the response returns.

The `voice-live` transport streams microphone PCM16 chunks over WebSocket while the PTT key is held, commits the audio buffer when recording ends, then uses Voice Live input transcription events for partial and final text. This avoids uploading the full utterance only after release, but it currently supports only `mai-transcribe-1`.

## Credentials

MAI credentials use `mai_credentials.json`:

```json
{
  "endpoint": "https://YOUR_RESOURCE_NAME.cognitiveservices.azure.com",
  "key": "your_speech_key"
}
```

Configure interactively:

```powershell
cargo run --bin configure_credentials -- --provider mai
```

Credential resolution order:

1. CLI arguments: `--mai-endpoint` and `--mai-key` for `voice_bridge`, or `--endpoint` and `--key` for `mai_demo`
2. Environment variables: `AZURE_SPEECH_ENDPOINT` and `AZURE_SPEECH_KEY`
3. `mai_credentials.json` in the current working directory or project root

The resolver also accepts nested Azure resource JSON with `/endpoints/resource_endpoint` and `/credentials/primary_key`. The public template should remain the simple `endpoint` + `key` shape above.

## Run

Run the bridge with MAI 1.5:

```powershell
cargo run --bin voice_bridge -- --asr-provider mai --mai-model mai-transcribe-1.5
```

Run the bridge with MAI 1:

```powershell
cargo run --bin voice_bridge -- --asr-provider mai --mai-model mai-transcribe-1
```

Run the bridge with MAI 1 through Voice Live:

```powershell
cargo run --bin voice_bridge -- --asr-provider mai --mai-transport voice-live --mai-model mai-transcribe-1
```

Run a one-shot file test:

```powershell
cargo run --bin mai_demo -- --audio-file .\audio.wav --model mai-transcribe-1.5
```

Run a fixed-duration Voice Live microphone test:

```powershell
cargo run --bin mai_demo -- --input-source mic --transport voice-live --model mai-transcribe-1 --mic-duration 5
```

Use environment variables instead of a credentials file:

```powershell
$env:AZURE_SPEECH_ENDPOINT = "https://YOUR_RESOURCE_NAME.cognitiveservices.azure.com"
$env:AZURE_SPEECH_KEY = "your_speech_key"
cargo run --bin voice_bridge -- --asr-provider mai --mai-model mai-transcribe-1.5
```

## Model Options

Common MAI options:

- `--mai-transport rest|voice-live`
- `--mai-model mai-transcribe-1|mai-transcribe-1.5`
- `--mai-locale <locale>`: optional locale hint; repeat or comma-separate values.
- `--mai-timeout <seconds>`: request/final-result timeout.
- `--mai-max-retries <count>`: retry count for 429 and transient 5xx responses.

When no locale is provided, the bridge omits the locale hint and lets MAI auto-detect speech languages. This is the default and is usually preferable for Chinese/English code-switching.

MAI 1.5-only options:

- `--mai-style verbatim`
- `--mai-phrase <phrase>`: repeat to add phrase/entity bias.

Example:

```powershell
cargo run --bin voice_bridge -- --asr-provider mai --mai-model mai-transcribe-1.5 --mai-phrase "Azure AI Foundry"
```

`phraseList` is entity bias, not a free-form prompt or long dialog context. MAI does not support the Doubao `--asr-auto-context` dialog context path in this integration.

Voice Live options:

- `--mai-live-api-version <version>`: Voice Live API version. Defaults to `2026-04-10`.
- `--mai-live-model <model>`: Voice Live session model. Defaults to `gpt-4.1`.
- `--mai-live-turn-detection none|server_vad|azure_semantic_vad|azure_semantic_vad_multilingual`: server-side turn detection. Defaults to `none` because PTT release is the local end-of-utterance signal.
- `--mai-live-silence-duration-ms <ms>`: Voice Live silence duration when turn detection is enabled. Defaults to `500`.

`voice-live` streams audio while recording, but `mai-transcribe-1.5` remains REST-only in this code path. If you select `--mai-transport voice-live`, also select `--mai-model mai-transcribe-1`.
