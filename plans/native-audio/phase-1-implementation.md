# Native audio proof of concept implementation plan

Goal: demonstrate native reuse without changing the production WASM engine.

Architecture: a separate `native-render` workspace member uses the existing
`dsp::osc::BandLimitedOscillator` and `engine_env::AudioBusProcessor`. Standard
Rust file I/O writes stereo PCM16 WAV offline. It does not claim native project
playback: device dispatch and the 32-bit offset ABI need a separate adaptation.

Constraints: preserve browser builds; no plugin hosting, audio driver, desktop
shell, project import, or production ABI changes; use the feature branch only.

1. Add the crate manifest, failing renderer contract, and integration tests.
   Check WAV metadata, empty/tail blocks, analytical signal/energy, deterministic
   bytes, RIFF overflow and propagated I/O errors with `cargo test -p native-render`.
2. Implement `render_wav(writer: impl Write, frames: u32) -> io::Result<()>` using
   a persistent oscillator, a single-source existing summing bus and fixed-size
   quantum scratch. Write 48 kHz, 1 kHz, gain 0.2, dual-mono PCM16.
3. Add a CLI taking an output path and optional frame count (default 48000),
   refusing to overwrite existing files. Test successful file creation and errors.
4. Validate Rust workspace tests, the existing WASM build and relevant WASM tests.
   Record unavailable checks precisely; document architecture and hosting steps.
5. Review the diff, commit on `feature/native-audio-engine-poc`, publish and open
   a PR if the configured GitHub identity permits it. Never merge.

Review focus: resetting phase per block; emitting padded tail frames; truncating
RIFF sizes; silently hiding write errors; accidental overwrite of user files.
Each has a corresponding renderer or CLI test. Implement inline in this session.
