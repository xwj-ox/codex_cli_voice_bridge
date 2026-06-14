# Windows release bundle

## Contents

- `voice_bridge.exe`: push-to-talk bridge binary.
- `doubao_asr_demo.exe`: one-shot Doubao ASR demo.
- `mai_demo.exe`: one-shot MAI demo.
- `configure_credentials.exe`: local credential setup helper.
- `voice_bridge_doctor.exe`: runtime environment check helper.
- `*.cmd`: launchers that run the matching binary from this folder.
- `doubao_credentials.example.json` and `doubao_credentials.json`.
- `mai_credentials.example.json` and `mai_credentials.json`.
- `docs\providers\`: full provider setup notes.

## First Run

1. Choose a provider: `doubao` or `mai`.
2. Configure credentials:

```cmd
configure_credentials.cmd --provider doubao
configure_credentials.cmd --provider mai
```

3. Start the bridge:

```cmd
voice_bridge.cmd --asr-provider doubao
voice_bridge.cmd --asr-provider mai --mai-model mai-transcribe-1.5 --mai-locale zh
voice_bridge.cmd --asr-provider mai --mai-transport voice-live --mai-model mai-transcribe-1 --mai-locale zh
```

4. Hold the configured PTT key to talk, then release it to recognize and paste the final text.

## Provider Docs

- Doubao setup: `docs\providers\doubao.md`
- MAI setup: `docs\providers\mai.md`

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

```cmd
doubao_asr_demo.cmd --audio-file audio.pcm --audio-format pcm
mai_demo.cmd --audio-file audio.wav --locale zh --model mai-transcribe-1.5
mai_demo.cmd --input-source mic --transport voice-live --model mai-transcribe-1 --locale zh --mic-duration 5
```

## Notes

- Windows default PTT key is `capslock`.
- Windows supports `space`, `enter`, `capslock`, `left-win`, `right-control`, `right-shift`, `f1` through `f12`, or a single letter as the PTT key.
- Windows does not support `fn` as the PTT key because Fn is not exposed as a standard Windows virtual key.
- If the target application is running as Administrator, run `voice_bridge.exe` with matching privileges.
- Microphone access must be enabled in Windows Privacy & security settings.
- The target window is captured on the initial key-down, then the final text is pasted back into that app.
- If you distribute the bundle to another machine, that machine still needs its own credentials and microphone setup.
