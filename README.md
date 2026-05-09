# codex_cli_voice_bridge_rust

Minimal public repository for the Rust implementation of the Doubao ASR CLI and the Codex-style push-to-talk voice bridge.

## Included

- `src/`: Rust source
- `.github/workflows/macos-compile-check.yml`: macOS compile-check CI
- `packaging/build_windows_release.ps1`: Windows release bundle script
- `packaging/build_macos_release.sh`: macOS release bundle script
- `packaging/check_macos_ready.sh`: macOS environment check
- `doubao_credentials.example.json`: blank credentials template

## Not included

- Real credentials
- Local logs
- Built artifacts
- Release bundles

## Current status

- Windows build path is implemented and locally verified
- macOS build path is implemented and locally package-verified; runtime still requires the host app to have macOS privacy permissions

## Quick start

Configure credentials:

```powershell
cargo run --bin configure_credentials
```

List input devices:

```powershell
cargo run --bin doubao_asr_demo -- --mic-list-devices
```

Run the bridge:

```powershell
cargo run --bin voice_bridge
```

## Doubao service setup

This project calls the Doubao big model streaming ASR WebSocket API:

- Default endpoint: `wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async`
- Default resource ID: `volc.seedasr.sauc.duration`
- Auth headers sent by this project: `X-Api-App-Key`, `X-Api-Access-Key`, `X-Api-Resource-Id`, and `X-Api-Connect-Id`

Set up the service in Volcengine before running the bridge:

1. Register or log in to a Volcengine account and complete real-name verification if the console requires it.
2. Open the Doubao Speech console: https://console.volcengine.com/speech/app
3. Use the old console for this project. The current Rust bridge uses the APP ID / Access Token WebSocket authentication path, not the new console API Key path.
4. In `应用中心` > `应用管理`, create an application or select an existing one.
5. Click `编辑` for the application and enable `豆包流式语音识别模型2.0 小时版` for the default resource ID. Use `豆包流式语音识别模型2.0 并发版` only if you want the concurrency resource ID.
6. Open `API服务中心` > `豆包流式语音识别模型2.0`.
7. In `服务接口认证信息`, copy `APP ID` and `Access Token`. The page also shows `Secret Key`, but this project does not use it.
8. Put the APP ID and Access Token into this project with `configure_credentials`, `doubao_credentials.json`, environment variables, or CLI arguments.

The new console banner says API access uses API Key and the old console uses APP ID. Do not paste a new-console API Key into `access_token`; it is a different authentication method and would require code changes.

Resource ID choices:

- ASR 2.0 hour-based: `volc.seedasr.sauc.duration` (project default)
- ASR 2.0 concurrency-based: `volc.seedasr.sauc.concurrent`
- ASR 1.0 hour-based: `volc.bigasr.sauc.duration`
- ASR 1.0 concurrency-based: `volc.bigasr.sauc.concurrent`

If you enable a concurrency package instead of an hour-based package, pass the matching resource ID:

```powershell
cargo run --bin voice_bridge -- --resource-id volc.seedasr.sauc.concurrent
```

Official references:

- New console quick start: https://www.volcengine.com/docs/6561/2119699
- Old console quick start: https://www.volcengine.com/docs/6561/163043
- Doubao Speech console FAQ: https://www.volcengine.com/docs/6561/196768
- Big model streaming ASR API: https://www.volcengine.com/docs/6561/1354869

The console may first show trial quota. For production or exhausted trial quota, open the formal paid service or buy the needed resource package in Volcengine. If the service is not opened for the selected application, or the resource ID does not match the opened package, the WebSocket handshake can fail even when the APP ID and Access Token are correct.

## Credentials

Credentials are resolved in this order:

1. CLI arguments: `--app-id` and `--access-token`
2. Environment variables: `DOUBAO_ASR_APP_ID` and `DOUBAO_ASR_ACCESS_TOKEN`
3. `doubao_credentials.json` in the current working directory or project root

Paste the raw APP ID and Access Token values from the Doubao Speech console. Do not add `Bearer`, `Bearer;`, or any other prefix to the Access Token.

The simplest setup is the interactive helper:

```powershell
cargo run --bin configure_credentials
```

That writes `doubao_credentials.json`:

```json
{
  "app_id": "your_app_id",
  "access_token": "your_access_token"
}
```

Use environment variables if you do not want to create a JSON file.

Windows PowerShell:

```powershell
$env:DOUBAO_ASR_APP_ID = "your_app_id"
$env:DOUBAO_ASR_ACCESS_TOKEN = "your_access_token"
cargo run --bin voice_bridge
```

Persist the variables for future PowerShell windows:

```powershell
[Environment]::SetEnvironmentVariable("DOUBAO_ASR_APP_ID", "your_app_id", "User")
[Environment]::SetEnvironmentVariable("DOUBAO_ASR_ACCESS_TOKEN", "your_access_token", "User")
```

Open a new PowerShell window after setting persistent environment variables.

Windows cmd.exe:

```cmd
set DOUBAO_ASR_APP_ID=your_app_id
set DOUBAO_ASR_ACCESS_TOKEN=your_access_token
cargo run --bin voice_bridge
```

macOS/Linux shells:

```bash
export DOUBAO_ASR_APP_ID="your_app_id"
export DOUBAO_ASR_ACCESS_TOKEN="your_access_token"
cargo run --bin voice_bridge
```

Or pass credentials directly for one run:

```bash
cargo run --bin voice_bridge -- --app-id "your_app_id" --access-token "your_access_token"
```

## Defaults

- `voice_bridge` enables `enable_nonstream` by default
- Windows default PTT key: `capslock`
- Windows supported PTT keys: `space`, `enter`, `capslock`, `left-win`, `right-control`, `right-shift`, `f1` through `f12`, or a single letter. Windows does not support `fn` as the PTT key because Fn is not exposed as a standard Windows virtual key.
- macOS default PTT key: `capslock`
- On macOS, `capslock` remains a system special key. While it is used as the PTT key, it can still switch between non-Latin and Latin input sources, or trigger Caps Lock / continuous uppercase behavior.
- macOS also supports `fn` as a PTT key; when `fn` is used for PTT, Fn/Globe shortcuts and `fn+...` key combinations are unavailable while the bridge is running
- Default PTT hold threshold: `250 ms`
- Default maximum recording duration: `60 s`

## Contextual ASR (hotwords / corpus)

Doubao streaming ASR supports request-time hints for domain words (hotwords) and richer dialog context via `request.corpus`.

This bridge exposes them as optional CLI arguments:

- `--asr-hotword <word>`: repeat to add multiple hotwords (serialized into `corpus.context` as a hotwords JSON string)
- `--asr-corpus-context <json>`: advanced mode; passes the provided string directly as `corpus.context` (when set, `--asr-hotword` is ignored)
- `--asr-boosting-table-id` / `--asr-boosting-table-name`: reference a hotword table configured in the Doubao self-learning platform
- `--asr-correct-table-id` / `--asr-correct-table-name`: reference a replacement table configured in the self-learning platform
- `--asr-auto-context`: maintain a short dialog context across utterances (serialized as a `dialog_ctx` payload in `corpus.context`)

Example: a few hotwords:

```powershell
cargo run --bin voice_bridge -- --asr-hotword "希沃白板" --asr-hotword "OfficePLUS"
```

Example: pass a full `corpus.context` JSON payload:

```bash
cargo run --bin voice_bridge -- --asr-corpus-context '{"hotwords":[{"word":"希沃白板"}]}'
```

The `corpus.context` value is sent to the ASR service with each utterance. Avoid including sensitive text in the context payload.

When `--asr-auto-context` is enabled, the bridge also sends recent final recognition texts as the dialog context for future utterances. This can improve recognition continuity, but it also means prior dictation text is sent along with each new utterance.

## Release helpers

Windows:

```powershell
powershell -ExecutionPolicy Bypass -File .\packaging\build_windows_release.ps1
```

macOS:

```bash
bash ./packaging/check_macos_ready.sh
bash ./packaging/build_macos_release.sh
```

## Security note

Do not commit real credentials. If you use a file, keep `doubao_credentials.json` local and use `doubao_credentials.example.json` only as the template.
