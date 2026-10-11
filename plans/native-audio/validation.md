# Phase 1 validation

Platform: Linux x86_64, Node 24.19.0, stable Rust 1.99.0 and nightly
1.101.0 (2026-10-11). Toolchains, baseline snapshot, logs and generated WAVs
were placed under `/workspace/work`, outside the repository.

## Passing checks

| Check | Result |
| --- | --- |
| `cargo test -p native-render` | 8 tests passed (5 renderer, 3 CLI). |
| `cargo test -p native-render --release` | The same 8 tests passed. |
| `cargo fmt -p native-render -- --check` | Passed. |
| `cargo clippy -p native-render --all-targets --no-deps -- -D warnings` | Passed. |
| Debug and release CLI rendering plus `cmp` | Byte-identical one-second WAV files. |
| Python standard-library `wave` decoder | 48000 frames, 48000 Hz, stereo, 16-bit PCM. |
| `npm run build:wasm` in `packages/studio/core-wasm` | Fresh engine, stretch analysis module and all stock PIC device modules built and copied successfully. |
| `npm run build:bundles` in that package | AudioWorklet and offline-worker JS bundles built. |
| `cargo test --workspace --exclude studio-boxes` | 930 passed, 0 failed; all remaining workspace packages completed. |
| Convolver benchmark rerun alone | 1 test passed with the freshly built WASM. |
| Proxy-enabled loudness/live-meter rerun | 57 loudness tests and 2 live-meter tests passed. |

Renderer tests verify WAV format/metadata/RIFF sizes, zero frames, 1/127/128/129
frames and full-second output, equal stereo channels, an independent analytical
1 kHz sine reference for every sample, expected energy, repeated bytes and prefix
consistency, RIFF overflow before any writes and I/O error propagation. CLI tests
verify actual files against library output, default/tail counts, invalid arguments
and preservation of existing files. Tests were first run against missing renderer
and CLI behavior before implementation.

The debug/release WAV SHA-256 on this target is
`b378d20122e51a5ced19784f12f4194fa712e4988e9cd7533218ead832342d24`.
This records same-target consistency, not a universal floating-point identity
promise across architectures or future DSP changes.

## Full-suite failures and environment adjustments

`cargo test --workspace` first failed compiling the existing Signalsmith C++
oracle: bindgen's Clang could not find `stddef.h`. GCC's installed header was
found at `/usr/lib/gcc/x86_64-linux-gnu/14/include`. Re-running with
`BINDGEN_EXTRA_CLANG_ARGS='-isystem /usr/lib/gcc/x86_64-linux-gnu/14/include'`
resolved this environment issue without repository edits.

The subsequent full run completed 843 passing tests before failing
`studio-boxes/tests/all_boxes_fixture.rs::all_boxes_round_trip_byte_identical`:
re-encoded graph bytes differ from `test-files/all-boxes.od`. The other two tests
in that integration file passed. The same test failure was reproduced from a
`git archive` snapshot of the unchanged base commit `0c66d11f9`; it predates this
change. Cargo stops at the failed binary, so the additional workspace run excluded
the entire `studio-boxes` package to exercise the remaining packages. That run is
not a claim that the full workspace passes.

`npm run typecheck` (`tsc --noEmit`) failed in unchanged dependency sources:

- `core/src/BufferUnderrunDetector.ts:11,17`: missing `AudioPlaybackStats`.
- `core/src/capture/CaptureAudio.ts:248`: `MediaTrackSettings.latency` missing.

No browser type declarations or TypeScript source were changed for this PoC.
The app-wide build/test pipeline was not claimed passing; validation focuses on
the affected Rust workspace and production WASM module/bundle paths.

The initial core-wasm Vitest run on prebuilt artifacts reported 423 passed,
7 failed, 3 skipped, and one worker RPC timeout. Failures were the private-path
test below, three asset-fetch cases, and three 5-second loudness timeouts.
The **freshly rebuilt** full run (`npm run test:vitest -- --maxWorkers=2`)
reported **425 passed, 5 failed, 3 skipped** (102 passing / 4 failing / 3 skipped
files). Its failures were:

- `convolver-bench.test.ts`: p99 2448 µs exceeded the 1333 µs half-quantum budget
  during concurrent cloud validation. Rerun alone with one worker passed; cloud
  timing results are not a hardware realtime guarantee.
- `live-meter-teardown-fuzz.test.ts`: both session-fuzz and full-output/broadcast
  finite-value tests failed fetching assets (`EAI_AGAIN assets.opendaw.studio`).
- `freeze-80s-full.test.ts`: full-song per-unit freeze finite-value test failed
  for the same asset DNS error.
- `hit-od-render.test.ts`: missing private fixture at
  `/Users/am/Repositories/andre.michelle/openDAW/test-files/probe/synthlib/hit.od`.
  Its output path is also a private macOS Downloads directory. This diagnostic
  cannot run as written in Linux cloud; no fake fixture was substituted.

Node fetch does not automatically use the session HTTP proxy. A further run used
`NODE_USE_ENV_PROXY=1`, one worker and `--testTimeout=30000` for loudness,
live-meter and full-freeze tests. It uses the configured proxy without bypassing
network policy. Loudness and both live-meter cases passed. The full-freeze rerun
successfully fetched assets and began its 104792-quantum render per instrument.
It was stopped after several minutes of sustained CPU work without completing
the first unit; there is no passing assertion result for that rerun. This costly
full-song diagnostic remains unverified, rather than counted as a success.

`wasm-opt` is not installed; the existing build script explicitly permits
unoptimized artifacts. No linker flags, device list or browser build scripts were
changed. All WASM package builds and the tested device loading/rendering paths
continue to work; this does not establish an interactive browser smoke test.

## Review, changed files and remaining limits

A separate code reviewer found no blocking implementation issues. A documented
minor limitation is that write/flush failure may leave a partial *new* WAV; the
CLI refuses overwrites. The reviewer also corrected hosting-plan wording to
distinguish transaction rejection from separate checksum verification/recovery.

Changed files: workspace `crates/Cargo.toml`/`Cargo.lock` (one new local member,
no dependency version changes); `crates/native-render` manifest, renderer, CLI,
two integration-test files and README; four documents in `plans/native-audio`.
No existing DSP, engine, ABI, TypeScript or production WASM code was modified.

Not exercised: interactive browser/Web Audio playback; native audio hardware,
latency/underruns, Windows/macOS targets, plugin binaries and native editor windows,
Tauri integration, native full-project playback. The core blockers remain
instance extraction, typed device dispatch and removal of 32-bit WASM pointer
assumptions from the native boundary. CLAP and VST3 remain design-only.
