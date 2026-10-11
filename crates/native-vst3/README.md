# Native VST3 offline adapter

Linux x86_64 offline plugin evaluation, separate from the browser engine. Uses
`vst3-host = 0.10.0` with default features disabled, and a real Rust VST3 fixture
using maintained, generated `vst3 = 0.3.0` SDK bindings. No hand-written COM ABI.
The published host is vendored with a narrow child-before-parent interface
release-order patch; see `vendor/README.md` and `vendor/release-order.patch`.

This is **offline only**: the upstream host locks mutexes in its process path,
and this adapter allocates/sorts events and resizes buffers. Do not put it in a
hardware audio callback. An RT-safe graph adapter requires a separately audited
implementation with preallocated buses/events and bounded command queues.

## API

- `list_plugins(path)` executes discovery and returns every audio class with its
  class ID, name, vendor and version; a module may export multiple audio classes.
- `load(path, id, sample_rate, max_frames)` creates a same-thread owned instance.
  Supports zero/one active stereo input bus and one active stereo output bus.
  Unsupported layouts return an error; sidechains are not silently flattened.
- `parameters()` returns parameter metadata and normalized `0..=1` values.
  Parameter identity is the VST3 parameter ID, not enumeration order.
- `process([left_in, right_in], [left_out, right_out], events)` processes one
  nonempty block no larger than `max_frames`. `Event` mirrors the native CLAP
  shape: sample-offset parameter points or three-byte MIDI channel messages.
  All events validate before anything is queued; stable sorting preserves order
  at equal offsets. Exact next-block offsets must be queued in the next block.
- MIDI event bus zero supports notes, poly pressure and mapped CC/channel
  pressure/pitch bend. Other buses, malformed bytes, system messages, program
  changes and unmapped controllers return explicit errors. This interface is
  MIDI 1 only; it does not preserve explicit note IDs/MPE/MIDI 2 expressions.
- `save_state()` / `load_state(bytes)` stop processing and delegate both component
  and controller streams to the upstream versioned state envelope. Preserve the
  plugin class ID with stored bytes. State blobs are capped at 16 MiB; save checks
  the size after upstream collection, not while the plugin writes.
- `latency_samples()` / `tail_samples()` expose plugin-reported values. This
  adapter does not implement graph PDC.

Instances are deliberately `!Send`/`!Sync`; construct, process, restore and drop
on the same thread. Processing starts at the first block and remains active
across adjacent blocks, preserving plugin voices/tails. State operations stop
processing; the next block starts it again. The upstream host performs required
suspend/deactivate/resume transitions for I/O/latency restart requests outside
plugin `process` calls. Processing errors and instance destruction also stop it.
Discovery/loading executes arbitrary native plugin code in-process. Use the
external scanner helper for crash isolation; this adapter itself cannot contain
a plugin crash or hang. Editor hosting and device I/O are out of scope.

Linux convention roots are `~/.vst3`, `/usr/lib/vst3`, `/usr/local/lib/vst3`, plus
user-configured paths. Directory bundles resolve
`Contents/x86_64-linux/<binary>.so`; single-file shared libraries also load.

## Verification

Install a Rust toolchain supporting the dependency's Rust 1.85 minimum and Linux
`libxcb` development libraries (for example `libxcb1-dev`). The upstream Linux
editor dependency links XCB even with its default features disabled.

Run `cargo test -p native-vst3` from the `crates` workspace. Integration tests
build the separately scoped `fixtures/gain` cdylib in the temporary directory,
load its real factory, and exercise stereo automation offsets, MIDI offsets,
component/controller state, rejected events, independent lifecycles, short
blocks, adjacent-block voice continuity, and Linux bundle resolution. No
production plugins are needed.

Optional third-party release-order regression (binaries remain external):

```sh
OPENDAW_VST3_TEST_PLUGIN=/path/to/DragonflyHallReverb.vst3 \
  cargo test -p native-vst3 third_party_plugin_teardown -- --nocapture
```

The test runs three load/process/drop cycles in a subprocess and rejects native
child-interface release-order diagnostics. Without that environment variable,
this optional smoke test reports that it skipped third-party coverage.

Build a reusable fixture with:

```sh
cargo build --manifest-path native-vst3/fixtures/gain/Cargo.toml
```

Its library is `fixtures/gain/target/debug/libopendaw_vst3_fixture.so`, unless
`CARGO_TARGET_DIR` overrides the target directory. It is a gain effect whose MIDI
note-on sets a transient voice gain to velocity and note-off sets it to zero;
starting processing resets voice gain without resetting the persisted gain
parameter. This makes event timing and processing discontinuities observable.

The fixture is adapted from `coupler-rs/vst3-rs`'s gain example under
MIT OR Apache-2.0; both licenses are retained beside the fixture manifest.
The host dependency is MIT; generated bindings are MIT OR Apache-2.0. Binding
version 0.3.0 ships generated source and requires no SDK installation/build-time
header generation. The current Steinberg SDK uses MIT, but pin/review any SDK
revision separately if adding external SDK fixtures or distribution assets.
