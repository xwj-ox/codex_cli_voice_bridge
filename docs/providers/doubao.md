# Doubao Provider

Doubao is the default ASR provider. It uses the Doubao big model streaming ASR WebSocket API.

## Service

- Default endpoint: `wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async`
- Default resource ID: `volc.seedasr.sauc.duration`
- Auth headers sent by the bridge: `X-Api-App-Key`, `X-Api-Access-Key`, `X-Api-Resource-Id`, and `X-Api-Connect-Id`

## Credentials

Doubao credentials use `doubao_credentials.json`:

```json
{
  "app_id": "your_app_id",
  "access_token": "your_access_token"
}
```

Configure interactively:

```powershell
cargo run --bin configure_credentials -- --provider doubao
```

Credential resolution order:

1. CLI arguments: `--app-id` and `--access-token`
2. Environment variables: `DOUBAO_ASR_APP_ID` and `DOUBAO_ASR_ACCESS_TOKEN`
3. `doubao_credentials.json` in the current working directory or project root

Do not add `Bearer`, `Bearer;`, or any other prefix to the Access Token.

## Run

Run the bridge:

```powershell
cargo run --bin voice_bridge -- --asr-provider doubao
```

Run a one-shot file test:

```powershell
cargo run --bin doubao_asr_demo -- --audio-file .\audio.pcm --audio-format pcm
```

If you enabled a concurrency package instead of an hour-based package, pass the matching resource ID:

```powershell
cargo run --bin voice_bridge -- --resource-id volc.seedasr.sauc.concurrent
```

## Resource IDs

- ASR 2.0 hour-based: `volc.seedasr.sauc.duration` (default)
- ASR 2.0 concurrency-based: `volc.seedasr.sauc.concurrent`
- ASR 1.0 hour-based: `volc.bigasr.sauc.duration`
- ASR 1.0 concurrency-based: `volc.bigasr.sauc.concurrent`

## Contextual ASR

Doubao supports request-time hints and richer dialog context through `request.corpus`.

Bridge options:

- `--asr-hotword <word>`: repeat to add multiple hotwords.
- `--asr-corpus-context <json>`: pass a raw `corpus.context` JSON string; when set, `--asr-hotword` is ignored.
- `--asr-boosting-table-id` / `--asr-boosting-table-name`: reference a hotword table configured in the Doubao self-learning platform.
- `--asr-correct-table-id` / `--asr-correct-table-name`: reference a replacement table configured in the self-learning platform.
- `--asr-auto-context`: maintain a short dialog context across utterances.

Examples:

```powershell
cargo run --bin voice_bridge -- --asr-hotword "OfficePLUS" --asr-hotword "希沃白板"
```

```bash
cargo run --bin voice_bridge -- --asr-corpus-context '{"hotwords":[{"word":"OfficePLUS"}]}'
```

When `--asr-auto-context` is enabled, recent final recognition texts are sent as dialog context for future utterances. Avoid including sensitive text in context payloads.

## Service Setup Notes

Set up the service in Volcengine before running the bridge:

1. Register or log in to a Volcengine account and complete real-name verification if the console requires it.
2. Open the Doubao Speech console: https://console.volcengine.com/speech/app
3. Use the old console for this project. The bridge uses APP ID / Access Token WebSocket authentication, not the new console API Key path.
4. In `应用中心` > `应用管理`, create an application or select an existing one.
5. Click `编辑` for the application and enable `豆包流式语音识别模型2.0 小时版` for the default resource ID. Use `豆包流式语音识别模型2.0 并发版` only if you want the concurrency resource ID.
6. Open `API服务中心` > `豆包流式语音识别模型2.0`.
7. In `服务接口认证信息`, copy `APP ID` and `Access Token`. The page also shows `Secret Key`, but this project does not use it.

The new console banner says API access uses API Key and the old console uses APP ID. Do not paste a new-console API Key into `access_token`; it is a different authentication method and requires code changes.

Official references:

- New console quick start: https://www.volcengine.com/docs/6561/2119699
- Old console quick start: https://www.volcengine.com/docs/6561/163043
- Doubao Speech console FAQ: https://www.volcengine.com/docs/6561/196768
- Big model streaming ASR API: https://www.volcengine.com/docs/6561/1354869
