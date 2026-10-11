# Linux native offline host

This executable renders the existing openDAW engine graph, loads native CLAP and
VST3 plugins, and processes WAV audio without a browser. It is an offline tool,
not a desktop DAW or a hardware audio driver.

## Build and test

Use Linux x86_64, Rust 1.85 or newer, and a C/C++ compiler. On Ubuntu/Debian:

```sh
sudo apt-get install build-essential pkg-config libxcb1-dev libclang-dev
cargo build --manifest-path crates/Cargo.toml -p native-host --release --locked
cargo test --manifest-path crates/Cargo.toml -p native-host -p native-clap -p native-vst3 -p native-render --locked
cargo test --manifest-path crates/Cargo.toml -p engine --features native --locked
```

Run these commands from the repository root. The executable is
`crates/target/release/native-host`. Tests build real CLAP/VST3 shared libraries
and load them dynamically; commercial plugins are not required for the tests.
The engine tests compare a production TypeScript project with its WASM render.
The existing Signalsmith oracle tests additionally need Clang's C++ include
paths configured correctly on the machine.

## Test a plugin

Generate a source using openDAW's existing oscillator:

```sh
cargo run --manifest-path crates/Cargo.toml -p native-render -- /tmp/source.wav 48000
crates/target/release/native-host list /path/to/Effect.clap
crates/target/release/native-host parameters /path/to/Effect.clap
crates/target/release/native-host render /path/to/Effect.clap /tmp/source.wav /tmp/clap-output.wav
```

For VST3, supply `--format vst3`. Linux bundles normally contain
`Contents/x86_64-linux/Plugin.so`; pass the bundle path:

```sh
crates/target/release/native-host list /path/to/Effect.vst3 --format vst3
crates/target/release/native-host parameters /path/to/Effect.vst3 --format vst3
crates/target/release/native-host render /path/to/Effect.vst3 /tmp/source.wav /tmp/vst3-output.wav --format vst3
```

Use `--id ID` if a module contains several plugins. Inspect `parameters` before
using `--param ID=VALUE`: CLAP values are in the plugin's plain range; VST3
values are normalized from zero to one. Read-only parameters are rejected.
Existing output files are never overwritten. Listen to the output WAV with
your usual player. WAV input supports mono/stereo PCM and float; output is
stereo PCM16 at the input's sample rate and exact frame count.

Save and restore opaque plugin state using `--state-out state.bin` and
`--state-in state.bin`. Keep the format, plugin ID/version and routing alongside
the blob; it is not an openDAW project attachment yet. State-output failure may
leave the successfully rendered WAV; use a fresh writable destination.

`--events events.json` supplies absolute sample positions:

```json
[
  {"type":"parameter","sample":64,"id":7,"value":0.5},
  {"type":"midi","sample":128,"port":0,"data":[144,69,100]},
  {"type":"midi","sample":24000,"port":0,"data":[128,69,0]}
]
```

Parameter IDs/ranges and MIDI input support depend on the plugin. Events are
ordered stably by sample, limited to 256 per 128-frame block, and must occur
inside the requested duration. The CLI does not add a plugin tail or compensate
latency; output has the same number of frames as the input.

The release CLI was also tested with Dragonfly Reverb 3.2.10 Linux x86_64:
Early Reflections, Hall, Plate and Room, in both CLAP and VST3 formats. Download
the official [Linux release](https://github.com/michaelwillis/dragonfly-reverb/releases/tag/3.2.10)
and pass its `.clap` file or `.vst3` bundle to the commands above. The VST3
dependency carries a narrow, documented interface-release-order patch.

## Render an openDAW project

```sh
crates/target/release/native-host render-project crates/engine/tests/fixtures/vaporisateur.od /tmp/project.wav --frames 48000
crates/target/release/native-host render-project crates/engine/tests/fixtures/vaporisateur.od /tmp/project-vst3.wav --frames 48000 --plugin /path/to/Effect.vst3 --format vst3
```

The native graph and optional master-output plugin execute in the same process.
The original OPEN/version-2 project envelope, bare BoxGraph snapshots and
version-1 ZIP project bundles are accepted. The native engine currently supports
Vaporisateur instruments, notes, value automation, timeline/transport, ordinary
audio buses and channel strips. Other devices or unsupported graph features
fail explicitly. Disable the output maximizer and remove unsupported devices
before exporting a project for this subset. This is not general project parity.
The optional plugin is a master-output stage, not a saved graph device.

## Discover installed plugins

```sh
crates/target/release/native-host scan --format clap
crates/target/release/native-host scan --format vst3
crates/target/release/native-host scan /custom/plugins --format vst3
```

Default roots are `~/.clap`, `/usr/lib/clap`, `/usr/local/lib/clap` or their VST3
equivalents. Discovery probes each candidate in a separate process with a
five-second timeout and bounded output, returning JSON with successes and errors.
Symlinks are skipped; pass the resolved path as a root. There is no persistent
discovery cache yet. Direct `list`, `parameters` and render commands execute
plugin code in their own process and may crash if the plugin crashes.

## Remaining work

Neither adapter should run in a realtime audio callback. VST3's maintained host
uses locks, and the CLI allocates between blocks. Plugin transport, native editor
windows, hardware playback, sidechains, graph insertion, plugin state embedded
in projects and delay compensation remain next increments. GUI and audio-device
validation require a Linux desktop with a display and an audio device.
See `/plans/native-audio` for investigation, decisions and the implementation plan.
