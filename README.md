# codex_cli_voice_bridge_rust

Rust push-to-talk voice bridge for pasting ASR output into the foreground app. The bridge has two first-class ASR providers:

- `doubao`: Doubao streaming ASR over WebSocket.
- `mai`: MAI through Azure Speech REST transcription or Voice Live WebSocket transcription.

Doubao remains the default provider. MAI is selected explicitly with `--asr-provider mai`.

## Quick Start

Configure credentials for the provider you want to use:

```powershell
cargo run --bin configure_credentials -- --provider doubao
cargo run --bin configure_credentials -- --provider mai
```

List input devices:

```powershell
cargo run --bin voice_bridge -- --mic-list-devices
```

Run the push-to-talk bridge:

```powershell
cargo run --bin voice_bridge -- --asr-provider doubao
cargo run --bin voice_bridge -- --asr-provider mai --mai-model mai-transcribe-1.5 --mai-locale zh
cargo run --bin voice_bridge -- --asr-provider mai --mai-transport voice-live --mai-model mai-transcribe-1 --mai-locale zh
```

Run one-shot file demos:

```powershell
cargo run --bin doubao_asr_demo -- --audio-file .\audio.pcm --audio-format pcm
cargo run --bin mai_demo -- --audio-file .\audio.wav --locale zh --model mai-transcribe-1.5
```

## Provider Setup

Use the provider docs for account setup, credential formats, and provider-specific ASR options:

- [Doubao provider](docs/providers/doubao.md)
- [MAI provider](docs/providers/mai.md)

Credential files are intentionally separate:

```text
doubao_credentials.json
mai_credentials.json
```

Both files are ignored by git. Only the `*.example.json` templates should be committed.

## Common Bridge Options

- `--asr-provider doubao|mai`: choose the ASR provider.
- `--ptt-key <key>`: push-to-talk key. Defaults to `capslock`.
- `--mic-device <selector>`: microphone device index or name substring.
- `--mic-duration <seconds>`: maximum recording duration for one utterance. Defaults to `60`.
- `--preview-mode single|line|off`: interim text preview mode.
- `--submit`: press Enter after pasting the final text.
- `--output-json <path>`: write per-utterance JSON results.

Provider-specific options keep their provider prefix where needed:

- Doubao: `--resource-id`, `--ws-url`, `--asr-hotword`, `--asr-corpus-context`, `--asr-auto-context`.
- MAI: `--mai-transport`, `--mai-model`, `--mai-locale`, `--mai-style`, `--mai-phrase`, `--mai-credentials`.

## Defaults

- `voice_bridge` uses `doubao` unless `--asr-provider mai` is passed.
- `voice_bridge` enables Doubao `enable_nonstream` by default.
- Windows default PTT key: `capslock`.
- macOS default PTT key: `capslock`.
- Windows supports `space`, `enter`, `capslock`, `left-win`, `right-control`, `right-shift`, `f1` through `f12`, or a single letter as the PTT key.
- macOS also supports `fn`; when `fn` is used for PTT, Fn/Globe shortcuts and `fn+...` key combinations are unavailable while the bridge is running.

## Release Helpers

Windows:

```powershell
powershell -ExecutionPolicy Bypass -File .\packaging\build_windows_release.ps1
```

macOS:

```bash
bash ./packaging/check_macos_ready.sh
bash ./packaging/build_macos_release.sh
```

Release bundles include the binaries, credential templates, launch scripts, and the `docs/` folder.

## Repository Contents

- `src/`: Rust source.
- `docs/providers/`: provider-specific setup and behavior notes.
- `docs/diagnostics/`: runtime review notes.
- `packaging/`: release bundle scripts and release README files.
- `doubao_credentials.example.json`: blank Doubao credentials template.
- `mai_credentials.example.json`: blank MAI credentials template.

## Security

Do not commit real credentials. Keep these files local:

```text
doubao_credentials.json
mai_credentials.json
```

Generated output and build artifacts are ignored:

```text
outputs/
target/
packaging/release/
```
