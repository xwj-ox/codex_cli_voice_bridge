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

1. Fill in `doubao_credentials.json` or run `configure_credentials.cmd`.
2. Start the bridge with `voice_bridge.cmd` or `start_voice_bridge.cmd`.
3. Hold the configured PTT key to talk, then release it to recognize and paste the final text.

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

You can also pass credentials directly for one run:

```cmd
voice_bridge.exe --app-id your_app_id --access-token your_access_token
```

Credential priority is CLI arguments, then environment variables, then `doubao_credentials.json`.

## Notes

- Windows default PTT key is `capslock`.
- If the target application is running as Administrator, run `voice_bridge.exe` with matching privileges; Windows can block input injection from a lower-privilege process into an elevated window.
- Microphone access must be enabled in Windows Privacy & security settings.
- The target window is captured at long-press time, then the final text is pasted back into that app.
- If you distribute the bundle to another machine, that machine still needs its own credential and microphone setup.
