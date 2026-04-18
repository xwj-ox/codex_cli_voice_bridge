# 2026-04-16 Runtime Review

This document captures the repository review conclusions that were confirmed by local inspection and parallel code review passes on 2026-04-16.

## Validation baseline

- Repository state during review: `main`, ahead of `origin/main` by one local commit, clean working tree.
- Local checks that completed successfully during review:
  - `cargo check --bin voice_bridge --bin voice_bridge_doctor`
  - `cargo test --no-run`
  - `./packaging/build_macos_release.sh`

## Confirmed fix candidates

### P1

1. Windows PTT hook installation can fail silently during startup.
   - File: `src/platform/windows/ptt.rs`
   - The hook thread reports its thread ID before `SetWindowsHookExW` is called.
   - If hook installation fails, the thread returns, but `KeyboardPttController::new` still returns `Ok(...)`.
   - User-visible effect: the bridge appears to start normally on Windows, but PTT never activates.

2. The bridge can paste a non-final ASR hypothesis.
   - Files: `src/asr.rs`, `src/bridge_core.rs`
   - Both ASR session paths keep copying the latest non-empty hypothesis into `final_text` before `got_final` becomes true.
   - `run_bridge_loop` previously treated any non-empty `final_text` as pasteable, even if `got_final == false`.
   - User-visible effect: an interim hypothesis can be pasted into the target window when the ASR session times out before a final result arrives.

### P2

3. macOS host-window protection seeds its blocklist from the current frontmost app.
   - File: `src/platform/macos/window.rs`
   - `MacosHostContext::detect()` previously added the frontmost app at bridge startup to the host blocklist.
   - User-visible effect: if the bridge is launched while an unrelated app is frontmost, every window from that app can be rejected for the rest of the session when `--forbid-host-window-target` is enabled.

4. macOS readiness preflight still requires removed shell dependencies.
   - File: `packaging/check_macos_ready.sh`
   - The script still checked `osascript` and `pbcopy` even though the macOS runtime no longer depends on either command after the native Pasteboard/Event API migration.
   - User-visible effect: false-negative readiness failures during setup or release validation.

5. macOS native paste injection currently overwrites the global clipboard without restoring it.
   - File: `src/platform/macos/paste.rs`
   - The native path writes dictated text into the system pasteboard and posts `Cmd+V`, but does not restore the previous clipboard contents.
   - User-visible effect: the user's clipboard is replaced after every successful paste.
   - This was confirmed as a real usability/runtime side effect, but was not yet selected for the first repair batch because it needs a careful native restoration path.

### P3

6. Target-window capture timing text does not match actual behavior.
   - Files: `src/bridge_core.rs`, `packaging/README_macos_release.md`, `packaging/README_windows_release.md`
   - The docs and startup banner said the target window is captured at long-press time.
   - Actual behavior: both macOS and Windows snapshot the foreground window on the initial key-down and reuse that value after the hold threshold expires.
   - User-visible effect: users can misunderstand what happens if they change focus while holding the PTT key.

## Confirmed improvement opportunities

These were intentionally separated from correctness bugs.

### Efficiency

1. PTT workers and microphone finalize logic still use polling loops.
   - Files: `src/platform/macos/ptt.rs`, `src/platform/windows/ptt.rs`, `src/audio.rs`
   - The current `recv_timeout` / `sleep` loops are simple and serviceable, but they keep small idle wakeups active and add minor release jitter.

2. Paste timing still depends on fixed sleeps.
   - Files: `src/platform/macos/paste.rs`, `src/platform/windows/paste.rs`
   - Refocus and paste sequencing currently wait conservative fixed delays.
   - Later optimization could replace some fixed waits with “continue immediately after foreground confirmation”.

3. `LinearMonoResampler::process` allocates temporary buffers on the hot path.
   - File: `src/audio.rs`
   - This is not a correctness issue, but future low-latency tuning can reuse scratch buffers to reduce allocation churn.

### Usability

1. “Recognized but not pasted” currently plays a success cue and then an error cue.
   - File: `src/bridge_core.rs`
   - This is a conflicting signal for the user and should eventually become its own state or cue path.

2. Paste-skip and paste-failure paths rely mainly on terminal output.
   - File: `src/bridge_core.rs`
   - A more explicit fallback path for preserved text would reduce friction when recognition succeeds but injection does not.

### UX / Docs

1. Platform key-behavior explanations are distributed across runtime output, the main README, and release READMEs.
   - Files: `src/bridge_core.rs`, `README.md`, `packaging/README_macos_release.md`, `packaging/README_windows_release.md`
   - This is workable now, but the content can drift over time.

2. Release READMEs are still thin on troubleshooting guidance.
   - Files: `packaging/README_macos_release.md`, `packaging/README_windows_release.md`
   - The runtime already exposes useful states such as `empty final text`, `no usable paste target captured`, and `paste failed`, but the release docs do not summarize them yet.

## First repair batch

The first repair batch selected from this review is:

1. Surface Windows PTT hook-install failure to startup.
2. Block paste/submit when no final ASR result was received.
3. Stop macOS host-window protection from using the startup frontmost app as a host proxy.
4. Remove obsolete shell-command checks from macOS readiness preflight.
5. Correct target-window capture timing text in runtime output and release docs.

The macOS clipboard-restoration issue remains tracked for a later patch because it needs a careful native read/restore path.
