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
- macOS source scaffolding is in place, but full build/runtime validation still requires a real macOS environment

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

## Defaults

- `voice_bridge` enables `enable_nonstream` by default
- Windows default PTT key: `capslock`
- macOS default PTT key: `right-control`
- Default PTT hold threshold: `250 ms`
- Default maximum recording duration: `60 s`

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

Do not commit `doubao_credentials.json`. Use `doubao_credentials.example.json` as the template for local setup.
