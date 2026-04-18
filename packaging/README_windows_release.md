# Windows release bundle

## Contents

- `voice_bridge.exe`: push-to-talk bridge binary
- `doubao_asr_demo.exe`: one-shot ASR demo binary
- `configure_credentials.exe`: helper for local credential setup
- `voice_bridge_doctor.exe`: runtime environment check helper
- `*.cmd`: convenience wrappers that run the matching binary from this folder
- `doubao_credentials.example.json`: blank credentials template
- `doubao_credentials.json`: local writable credentials file

## First run

1. Set up the Doubao Speech service and get the APP ID and Access Token.
2. Fill in `doubao_credentials.json` or run `configure_credentials.cmd`.
3. Start the bridge with `voice_bridge.cmd` or `start_voice_bridge.cmd`.
4. Hold the configured PTT key to talk, then release it to recognize and paste the final text.

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

```cmd
voice_bridge.exe --resource-id volc.seedasr.sauc.concurrent
```

Official references:

- New console quick start: https://www.volcengine.com/docs/6561/2119699
- Old console quick start: https://www.volcengine.com/docs/6561/163043
- Doubao Speech console FAQ: https://www.volcengine.com/docs/6561/196768
- Big model streaming ASR API: https://www.volcengine.com/docs/6561/1354869

The console may first show trial quota. For production or exhausted trial quota, open the formal paid service or buy the needed resource package in Volcengine. If the service is not opened for the selected application, or the resource ID does not match the opened package, the WebSocket handshake can fail even when the APP ID and Access Token are correct.

Paste the raw Access Token value. Do not add `Bearer`, `Bearer;`, or any other prefix.

## Credentials without JSON

The binaries also read credentials from environment variables:

```cmd
set DOUBAO_ASR_APP_ID=your_app_id
set DOUBAO_ASR_ACCESS_TOKEN=your_access_token
voice_bridge.exe
```

In PowerShell:

```powershell
$env:DOUBAO_ASR_APP_ID = "your_app_id"
$env:DOUBAO_ASR_ACCESS_TOKEN = "your_access_token"
.\voice_bridge.exe
```

Persist the variables for future PowerShell windows:

```powershell
[Environment]::SetEnvironmentVariable("DOUBAO_ASR_APP_ID", "your_app_id", "User")
[Environment]::SetEnvironmentVariable("DOUBAO_ASR_ACCESS_TOKEN", "your_access_token", "User")
```

Open a new PowerShell window after setting persistent environment variables.

You can also pass credentials directly for one run:

```cmd
voice_bridge.exe --app-id your_app_id --access-token your_access_token
```

Credential priority is CLI arguments, then environment variables, then `doubao_credentials.json`.

## Notes

- Windows default PTT key is `capslock`.
- Windows supports `space`, `enter`, `capslock`, `left-win`, `right-control`, `right-shift`, `f1` through `f12`, or a single letter as the PTT key.
- Windows does not support `fn` as the PTT key because Fn is not exposed as a standard Windows virtual key.
- If the target application is running as Administrator, run `voice_bridge.exe` with matching privileges; Windows can block input injection from a lower-privilege process into an elevated window.
- Microphone access must be enabled in Windows Privacy & security settings.
- The target window is captured on the initial key-down, then the final text is pasted back into that app.
- If you distribute the bundle to another machine, that machine still needs its own credential and microphone setup.
