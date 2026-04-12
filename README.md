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

## Credentials

Credentials are resolved in this order:

1. CLI arguments: `--app-id` and `--access-token`
2. Environment variables: `DOUBAO_ASR_APP_ID` and `DOUBAO_ASR_ACCESS_TOKEN`
3. `doubao_credentials.json` in the current working directory or project root

Use environment variables if you do not want to create a JSON file:

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
- macOS default PTT key: `capslock`
- macOS also supports `fn` as a PTT key; when `fn` is used for PTT, Fn/Globe shortcuts and `fn+...` key combinations are unavailable while the bridge is running
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

Do not commit real credentials. If you use a file, keep `doubao_credentials.json` local and use `doubao_credentials.example.json` only as the template.
