# macOS release bundle

## Contents

- `voice_bridge`: push-to-talk bridge binary
- `doubao_asr_demo`: one-shot ASR demo binary
- `mai_demo`: one-shot Azure MAI demo binary
- `configure_credentials`: helper for local credential setup
- `voice_bridge_doctor`: runtime permission check helper
- `*.sh`: convenience wrappers that run the matching binary from this folder
- `doubao_credentials.example.json`: blank credentials template
- `doubao_credentials.json`: local writable credentials file
- `mai_credentials.example.json`: blank Azure MAI credentials template
- `mai_credentials.json`: local writable Azure MAI credentials file

## First run

1. Run `./voice_bridge_doctor` to check Accessibility and Input Monitoring access.
2. If needed, grant the host app access in System Settings -> Privacy & Security.
3. Choose an ASR provider: Doubao or Azure MAI.
4. For Doubao, set up the Doubao Speech service and get the APP ID and Access Token.
5. For MAI, set up an Azure Speech resource and get the endpoint and key.
6. Fill in the matching credentials JSON or run `./configure_credentials --provider doubao` / `./configure_credentials --provider mai`.
7. Start the bridge with `./voice_bridge` or `./start_voice_bridge.sh`.

## Doubao service setup

This bundle calls the Doubao big model streaming ASR WebSocket API. The default resource ID is `volc.seedasr.sauc.duration`.

1. Register or log in to a Volcengine account and complete real-name verification if the console requires it.
2. Open the Doubao Speech console: https://console.volcengine.com/speech/app
3. Use the old console for this bundle. The current bridge uses the APP ID / Access Token WebSocket authentication path, not the new console API Key path.
4. In `应用中心` > `应用管理`, create an application or select an existing one.
5. Click `编辑` for the application and enable `豆包流式语音识别模型2.0 小时版` for the default resource ID. Use `豆包流式语音识别模型2.0 并发版` only if you want the concurrency resource ID.
6. Open `API服务中心` > `豆包流式语音识别模型2.0`.
7. In `服务接口认证信息`, copy `APP ID` and `Access Token`. The page also shows `Secret Key`, but this bundle does not use it.

The new console banner says API access uses API Key and the old console uses APP ID. Do not paste a new-console API Key into `access_token`; it is a different authentication method and would require code changes.

Resource ID choices:

- ASR 2.0 hour-based: `volc.seedasr.sauc.duration` (bundle default)
- ASR 2.0 concurrency-based: `volc.seedasr.sauc.concurrent`
- ASR 1.0 hour-based: `volc.bigasr.sauc.duration`
- ASR 1.0 concurrency-based: `volc.bigasr.sauc.concurrent`

If you enable a concurrency package instead of an hour-based package, pass the matching resource ID:

```bash
./voice_bridge --resource-id volc.seedasr.sauc.concurrent
```

Official references:

- New console quick start: https://www.volcengine.com/docs/6561/2119699
- Old console quick start: https://www.volcengine.com/docs/6561/163043
- Doubao Speech console FAQ: https://www.volcengine.com/docs/6561/196768
- Big model streaming ASR API: https://www.volcengine.com/docs/6561/1354869

The console may first show trial quota. For production or exhausted trial quota, open the formal paid service or buy the needed resource package in Volcengine. If the service is not opened for the selected application, or the resource ID does not match the opened package, the WebSocket handshake can fail even when the APP ID and Access Token are correct.

## Azure MAI setup

MAI uses Azure Speech LLM Speech synchronous transcription. The bundle exposes both `mai-transcribe-1` and `mai-transcribe-1.5` through `--mai-model`.

Minimal credentials:

```json
{
  "endpoint": "https://YOUR_RESOURCE_NAME.cognitiveservices.azure.com",
  "key": "your_speech_key"
}
```

Run a file test:

```bash
./mai_demo --audio-file audio.wav --locale zh --model mai-transcribe-1.5
```

Run the bridge with either MAI model:

```bash
./voice_bridge --asr-provider mai --mai-model mai-transcribe-1 --mai-locale zh
./voice_bridge --asr-provider mai --mai-model mai-transcribe-1.5 --mai-locale zh
```

`mai-transcribe-1.5` also supports `--mai-style verbatim` and `--mai-phrase phrase`. Phrase list is entity bias, not a free-form prompt.

## Credentials without JSON

The binaries also read credentials from environment variables:

```bash
export DOUBAO_ASR_APP_ID="your_app_id"
export DOUBAO_ASR_ACCESS_TOKEN="your_access_token"
./voice_bridge
```

Paste the raw Access Token value. Do not add `Bearer`, `Bearer;`, or any other prefix.

You can also pass them directly for one run:

```bash
./voice_bridge --app-id "your_app_id" --access-token "your_access_token"
```

Credential priority is CLI arguments, then environment variables, then `doubao_credentials.json`.

For MAI:

```bash
export AZURE_SPEECH_ENDPOINT="https://YOUR_RESOURCE_NAME.cognitiveservices.azure.com"
export AZURE_SPEECH_KEY="your_speech_key"
./voice_bridge --asr-provider mai --mai-model mai-transcribe-1.5 --mai-locale zh
```

MAI credential priority is CLI arguments, then environment variables, then `mai_credentials.json`.

## Notes

- On macOS, microphone permission is requested by the audio capture path when recording starts.
- If you use `capslock` as the PTT key, macOS can still switch between non-Latin and Latin input sources, or trigger Caps Lock / continuous uppercase while the bridge is running because `capslock` remains a system special key.
- If you configure `fn` as the PTT key, Fn/Globe shortcuts and `fn+...` key combinations are unavailable while the bridge is running.
- The target window is captured on the initial key-down, then the final text is pasted back into that app.
- If you distribute the bundle to another machine, that machine still needs its own privacy permission grants.
