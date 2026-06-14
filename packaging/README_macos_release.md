# macOS release bundle

## Contents

- `voice_bridge`: push-to-talk bridge binary.
- `doubao_asr_demo`: one-shot Doubao ASR demo.
- `mai_demo`: one-shot MAI demo.
- `configure_credentials`: local credential setup helper.
- `voice_bridge_doctor`: runtime permission check helper.
- `*.sh`: launchers that run the matching binary from this folder.
- `doubao_credentials.example.json` and `doubao_credentials.json`.
- `mai_credentials.example.json` and `mai_credentials.json`.
- `docs/providers/`: full provider setup notes.

## First Run

1. Run `./voice_bridge_doctor` to check Accessibility and Input Monitoring access.
2. If needed, grant the host app access in System Settings -> Privacy & Security.
3. Choose a provider: `doubao` or `mai`.
4. Configure credentials:

```bash
./configure_credentials.sh --provider doubao
./configure_credentials.sh --provider mai
```

5. Start the bridge:

```bash
./voice_bridge.sh --asr-provider doubao
./voice_bridge.sh --asr-provider mai --mai-model mai-transcribe-1.5 --mai-locale zh
```

## Provider Docs

- Doubao setup: `docs/providers/doubao.md`
- MAI setup: `docs/providers/mai.md`

Doubao credentials, in `doubao_credentials.json`:

```json
{
  "app_id": "your_app_id",
  "access_token": "your_access_token"
}
```

MAI credentials, in `mai_credentials.json`:

```json
{
  "endpoint": "https://YOUR_RESOURCE_NAME.cognitiveservices.azure.com",
  "key": "your_speech_key"
}
```

Credential priority is CLI arguments, then environment variables, then the local credentials JSON.

## One-Shot Tests

```bash
./doubao_asr_demo.sh --audio-file audio.pcm --audio-format pcm
./mai_demo.sh --audio-file audio.wav --locale zh --model mai-transcribe-1.5
```

## Notes

- On macOS, microphone permission is requested by the audio capture path when recording starts.
- If you use `capslock` as the PTT key, macOS can still switch between non-Latin and Latin input sources, or trigger Caps Lock / continuous uppercase while the bridge is running.
- If you configure `fn` as the PTT key, Fn/Globe shortcuts and `fn+...` key combinations are unavailable while the bridge is running.
- The target window is captured on the initial key-down, then the final text is pasted back into that app.
- If you distribute the bundle to another machine, that machine still needs its own privacy permission grants and credentials.
