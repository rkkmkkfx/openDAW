# Offline native CLAP host

`list_plugins(path)` enumerates owned descriptor strings from a native CLAP binary.
`load(path, plugin_id, sample_rate, max_frames)` returns an instance owned by its
creating thread. It cannot be sent or shared between threads. Use trusted native
plugins: their code executes in the host process without sandboxing.

`Instance::process([left_input, right_input], [left_output, right_output], events)`
uses planar f32 slices and returns the CLAP processing status. Zero audio inputs
(instruments) or one stereo input, and exactly one stereo output are accepted.
Other layouts are rejected. For an instrument pass `[&[], &[]]`. Blocks must have
1..=max_frames frames. Input audio is copied to owned buffers because CLAP plugins
receive mutable channel pointers. Transport is currently unavailable (`null`).

Events are plain-range parameters and MIDI 1 three-byte messages with offsets
inside the current block. The input capacity is 256; excess events, unknown
parameters, out-of-range values and unsupported MIDI ports return an error.
Events are stably sorted by offset, preserving caller order at equal offsets.
Callers must order note-off/parameter/note-on events at equal time themselves.
`output_events()` exposes up to 256 MIDI/parameter events from the latest block;
unsupported output event types, targeted per-note parameter values, out-of-block
timestamps and overflow return false to the plugin and
increment `rejected_output_events()`. CLAP notes, per-note expression, MIDI 2,
SysEx, parameter gestures/modulation and output note-end events are unsupported.

`parameters()` exposes ID/name/plain range/default/flags. `parameter_value(id)`
reads the plugin's current value. `save_state()` and `load_state(bytes)` use the
CLAP opaque streams while stopped and deactivated, then reactivate the instance.
State is bounded to 16 MiB. The caller stores plugin ID/version and routing
metadata alongside these bytes. A failed restore can leave the plugin's state
changed; it is reported to the caller.

Required host restart/process/callback requests use atomic flags. Main-thread
callbacks and restarts run through `service_main_thread()` before each block,
never inside the plugin process call. `process_requested()` consumes a wake
request. Processing stays started between adjacent blocks; callbacks/state and
restart stop it before performing main-thread work. The host implements CLAP
thread-check using logical audio/main scopes on the same offline owner thread.
The dynamic library, entry, host and callback data outlive plugin destruction.

Bindings are pinned to `clap-sys` 0.5.0 (MIT/Apache-2.0) and `libloading` 0.8.9
(ISC), rather than handwritten CLAP struct declarations. The host has no editor,
hardware output, aux buses, latency compensation, plugin crash isolation or
realtime guarantees. Additional host extensions are conservatively unsupported.
Only Linux x86_64 has been executed. Zero-input instrument support is accepted
by the API but not yet covered by the gain-effect fixture.

From `crates`, run `cargo test -p native-clap`. Integration tests explicitly
build a real CLAP gain fixture cdylib with Cargo in a separate temporary target
directory, then dynamically discover/load/process/save/restore/unload it. The
fixture asserts lifecycle thread scopes. Tests cover exact automation/MIDI
sample offsets, continuous processing, deferred callback/restart requests, output
event rejection and lossless non-UTF8 Unix plugin paths.
