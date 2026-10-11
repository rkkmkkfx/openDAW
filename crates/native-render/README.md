# Native offline rendering proof

From the repository root:

```sh
cargo run --manifest-path crates/Cargo.toml -p native-render -- /tmp/opendaw-tone.wav
cargo run --manifest-path crates/Cargo.toml -p native-render -- /tmp/opendaw-tail.wav 129
cargo test --manifest-path crates/Cargo.toml -p native-render
```

The output is 48 kHz stereo PCM16: a deterministic 1 kHz sine at gain 0.2,
using the existing Rust oscillator and summing bus. The optional frame count
defaults to 48000; zero produces an empty WAV. Existing files are never overwritten.
An I/O failure can leave a partial new file; remove it before retrying. Streaming
uses fixed quantum scratch rather than memory proportional to the file length.

This is an offline DSP/engine-component compatibility proof, not native project
playback. It deliberately avoids the engine's WASM device pointer/table ABI.
See [architecture](../../plans/native-audio/architecture.md) and
[hosting plan](../../plans/native-audio/plugin-hosting-plan.md).
