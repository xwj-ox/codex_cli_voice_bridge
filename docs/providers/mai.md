# MAI Provider

MAI uses Azure Speech LLM Speech synchronous transcription. In this repository, user-facing provider names use `mai`; Azure model IDs still use the fixed names `mai-transcribe-1` and `mai-transcribe-1.5`.

## Service

- Endpoint path: `/speechtotext/transcriptions:transcribe`
- Auth header: `Ocp-Apim-Subscription-Key`
- Default API version: `2025-10-15`
- Supported model IDs: `mai-transcribe-1`, `mai-transcribe-1.5`

MAI is synchronous, not streaming. The bridge records one utterance, wraps microphone PCM as WAV, sends the file to Azure, and pastes the final text when the response returns.

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
cargo run --bin voice_bridge -- --asr-provider mai --mai-model mai-transcribe-1.5 --mai-locale zh
```

Run the bridge with MAI 1:

```powershell
cargo run --bin voice_bridge -- --asr-provider mai --mai-model mai-transcribe-1 --mai-locale zh
```

Run a one-shot file test:

```powershell
cargo run --bin mai_demo -- --audio-file .\audio.wav --locale zh --model mai-transcribe-1.5
```

Use environment variables instead of a credentials file:

```powershell
$env:AZURE_SPEECH_ENDPOINT = "https://YOUR_RESOURCE_NAME.cognitiveservices.azure.com"
$env:AZURE_SPEECH_KEY = "your_speech_key"
cargo run --bin voice_bridge -- --asr-provider mai --mai-model mai-transcribe-1.5 --mai-locale zh
```

## Model Options

Common MAI options:

- `--mai-model mai-transcribe-1|mai-transcribe-1.5`
- `--mai-locale <locale>`: repeat or comma-separate locale hints.
- `--mai-timeout <seconds>`: HTTP request timeout.
- `--mai-max-retries <count>`: retry count for 429 and transient 5xx responses.

MAI 1.5-only options:

- `--mai-style verbatim`
- `--mai-phrase <phrase>`: repeat to add phrase/entity bias.

Example:

```powershell
cargo run --bin voice_bridge -- --asr-provider mai --mai-model mai-transcribe-1.5 --mai-locale zh --mai-phrase "Azure AI Foundry"
```

`phraseList` is entity bias, not a free-form prompt or long dialog context. MAI does not support the Doubao `--asr-auto-context` dialog context path in this integration.
