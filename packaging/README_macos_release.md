# macOS release bundle

## Contents

- `voice_bridge`: push-to-talk bridge binary
- `doubao_asr_demo`: one-shot ASR demo binary
- `configure_credentials`: helper for local credential setup
- `voice_bridge_doctor`: runtime permission check helper
- `*.sh`: convenience wrappers that run the matching binary from this folder
- `doubao_credentials.example.json`: blank credentials template
- `doubao_credentials.json`: local writable credentials file

## First run

1. Run `./voice_bridge_doctor` to check Accessibility and Input Monitoring access.
2. If needed, grant the host app access in System Settings -> Privacy & Security.
3. Fill in `doubao_credentials.json` or run `./configure_credentials`.
4. Start the bridge with `./voice_bridge` or `./start_voice_bridge.sh`.

## Credentials without JSON

The binaries also read credentials from environment variables:

```bash
export DOUBAO_ASR_APP_ID="your_app_id"
export DOUBAO_ASR_ACCESS_TOKEN="your_access_token"
./voice_bridge
```

You can also pass them directly for one run:

```bash
./voice_bridge --app-id "your_app_id" --access-token "your_access_token"
```

Credential priority is CLI arguments, then environment variables, then `doubao_credentials.json`.

## Notes

- On macOS, microphone permission is requested by the audio capture path when recording starts.
- If you configure `fn` as the PTT key, Fn/Globe shortcuts and `fn+...` key combinations are unavailable while the bridge is running.
- The target window is captured at long-press time, then the final text is pasted back into that app.
- If you distribute the bundle to another machine, that machine still needs its own privacy permission grants.
