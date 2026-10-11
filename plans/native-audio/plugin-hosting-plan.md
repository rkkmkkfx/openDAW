# Native plugin hosting implementation plan

Status: Phase 1 was design only. The Linux continuation now includes tested
offline CLAP and VST3 hosting; realtime and desktop stages remain planned.
Spec: [architecture](architecture.md). CLAP first, VST3 second.

## Linux continuation status

The continuation provides a typed native facade for the existing graph and
production Vaporisateur, real CLAP and VST3 offline adapters, WAV/project
rendering, opaque state, sample-offset MIDI/automation and isolated discovery.
See [execution ledger](native-host-implementation.md) and
[runnable commands](../../crates/native-host/README.md). The CLI's optional plugin
is a master-output stage in the graph's process, not a saved graph device.

The remaining sequence is general graph insertion/state attachments, then a
realtime-safe CLAP/control split and Linux audio driver, routing/PDC, editors and
desktop UI integration. VST3 offline support follows the validated CLAP offline
milestone; its realtime adapter remains separate because the maintained host
uses process-path locks. The detailed stages below describe those remaining gaps.

The native engine deliberately owns thread-local host contexts and cannot move
between threads. A callback backend cannot simply capture it in a `Send` driver
closure. Use an explicitly owned audio thread for initial driver work, or extract
all host context into instance-owned state before designing thread transfer.
Plugin lifecycle/control and editor operations must still meet each API's thread
rules; adding a hardware callback alone does not establish realtime safety.

## Architecture and invariants

Use one native process for the graph and plugin DSP, owned by a single audio
thread; retain the web UI as a control client. A native `EngineFactory` adapter
should preserve the current UI engine contract. Keep a separately tested WASM
adapter for the existing browser deployment. Do not put Tauri commands, JSON,
filesystem access, plugin scanning or window operations in the audio callback.

The native core must be instance-owned, with platform-neutral graph/transport,
typed buffers and device dispatch. Its native adapter uses normal Rust pointers
and format-specific plugin C APIs. The WASM adapter continues using the current
offset descriptors/table slots. Do not reinterpret a WASM descriptor as native
CLAP/VST3 memory. Evaluate direct C bindings versus maintained wrapper crates
against ABI coverage, license and platform support; pin versions once selected.

The audio thread owns the graph's existing `Rc`/`RefCell` handles; do not move
those across threads. Control preparation communicates bounded commands and
prepared data with explicit ownership transfer. Use retire queues to reclaim
old nodes and resources off the callback. Plugins' own realtime behavior is an
external risk even if the host avoids allocation.

## Increment 1: native engine core and stock-device parity

Files: split reusable instance code from `crates/engine/src/lib.rs` behind an
importable core; add a native backend crate. Introduce a typed host/device adapter
around `plugin_instrument.rs`, `plugin_audio_effect.rs`, `plugin_midi_effect.rs`
and `abi` without changing WASM wire layouts. Start with sine and one effect.

Interfaces: `NativeEngine::new(sample_rate: f32)`,
`apply_transaction(bytes: &[u8]) -> Result<(), EngineError>` and
`render_quantum(output: &mut AudioBuffer)`. Error type must distinguish invalid
transactions, unavailable devices and resource failures. Native callbacks use
borrowed slices/typed state, not table indices or `u32` pointers.

- [ ] Add failing fixtures for graph creation, note on/off, transport/looping,
  gain/pan and device insertion/removal; compare native and WASM renders/checksums.
- [ ] Extract reusable core and adapt the two stock devices; keep WASM exports as
  wrappers. Separate allocator, panic handler, JS callbacks and global context.
- [ ] Feed real serialized BoxGraph snapshots/forward transactions from existing
  TS tests; preserve transaction rejection, checksum verification and full resnapshot recovery.
- [ ] Render a project offline natively and verify deterministic frames, event
  order and sample resources. Verify every WASM package still builds/tests.

Gate: native project playback, not merely compilation. JS script devices, NAM
and WebCLAP get an explicit capability report/placeholder policy until native
adapters exist; never silently omit a project's instrument.

## Increment 2: bounded events, automation and state ownership

Files: native backend event/control modules; reuse `EventRecord`, `Block`,
`transport`, note collections and `param_automation.rs` semantics where appropriate.

Interfaces: queued events carry absolute sample time, stable note ID, port/channel,
event kind and payload; plugin parameter keys are `(format, plugin_id, param_id)`,
not discovery order. `process_block` receives preallocated audio buses, bounded
event slices, transport and sample-rate configuration.

- [ ] Test events at offset 0, offset 127, exact next-block boundary, same-time
  note-off/parameter/note-on, seeks/loops, late events and queue overflow.
- [ ] Define pulse ↔ sample mapping and native MIDI-device timestamp conversion.
  Preserve note identity, velocity and supported expression; declare unsupported
  MIDI 2/MPE features instead of dropping them silently.
- [ ] Translate normalized UI values to plugin plain ranges, stepped/enumerated
  values, gestures and modulation. Preserve the existing 10-pulse update clock;
  add higher-resolution automation as an explicit tested feature later.
- [ ] Preallocate event storage with documented capacity. On overflow, publish a
  counter and recover note state using a defined all-notes-off/reset policy; never
  block or allocate. Reject out-of-range offsets and parameter IDs safely.
- [ ] Prepare graph/resource changes outside the callback and publish at quantum
  boundaries. Test repeated edits/removal while audio runs; no callback frees,
  locks, panics, unbounded reconciliation or ownership races.

## Increment 3: CLAP offline host

Files: new `clap-host` crate and plugin-format adapter in native backend.
Keep the shared graph independent of CLAP-specific raw structs.

Interfaces: discovered descriptor → owned instance → activate/deactivate →
start/stop processing → process → destroy. Dynamic library ownership outlives
every factory, instance, callback and GUI. Host extensions must remain valid
through destruction. Explicitly model main-thread and audio-thread transitions.

- [ ] Build a tiny native CLAP fixture plugin (instrument and effect), load its
  entry/factory and validate CLAP version/plugin IDs before construction.
- [ ] Implement lifecycle/thread-check, audio/note ports, parameters, transport,
  events, state, latency and required host callbacks. Route restart/process/
  callback requests to their correct threads; never run main-thread work inline
  in `process`. Treat unsupported extensions conservatively.
- [ ] Test mono/stereo and aux/sidechain ports, zero inputs, sleeping/waking DSP,
  block-boundary automation, note lifecycle and render status handling.
- [ ] Render the fixture offline in the native graph and compare an analytical
  reference. Check unload/reload, failed activation and multiple plugin instances.
- [ ] Add state round-trip tests with plugin ID/version/schema, opaque bytes and
  host routing/parameter metadata. Save/restore on a permitted non-audio thread,
  suspending/quiescing processing where required. Persist blob references in the
  existing project model, support missing-plugin placeholders and failed restore.

Gate: deterministic offline CLAP instrument/effect rendering and state restoration
before hardware I/O. Existing WebCLAP parameter/state UX is a reference, not a
native ABI implementation.

## Increment 4: native realtime audio, MIDI and routing

Files: native runtime/audio-driver adapter and graph routing/PDC modules.
Select a cross-platform audio API after testing Linux ALSA/PipeWire/JACK and the
required macOS/Windows backend behavior. Do not assume 128-frame driver blocks.

- [ ] Adapt variable driver callback sizes to internal 128-frame quanta with
  preallocated FIFOs; test 64/128/256 and irregular counts, no dropped samples.
  Measure and expose adapter buffering latency, including sub-quantum callbacks.
- [ ] Route planar native plugin buses within the graph, preserving topology,
  sidechains, bypass, mute/solo, MIDI buses and explicit feedback/cycle policy.
- [ ] Implement delay compensation from node latency: at each merge align paths
  to their maximum cumulative arrival delay, including sidechains and MIDI events
  tied to delayed audio. Distinguish realtime monitoring policy and offline tails.
- [ ] Test parallel impulse paths, multi-hop latency, aux sends, plugin bypass,
  dynamic latency changes, instrument paths and stem alignment. Allocate delay
  lines/rebuild schedules off audio thread and swap them coherently.
- [ ] Implement timestamped native MIDI input/output, clock translation, bounded
  overflow behavior and resets on device disconnect/seek. Test MIDI/audio alignment.
- [ ] Instrument allocation/lock counters and underruns; measure callback CPU,
  percentile timing and loopback latency on actual hardware. Plugin CPU load can
  violate deadlines; report it rather than claiming a universal latency guarantee.

## Increment 5: scanning, discovery and native editors

Files: scanner helper executable, versioned cache/catalogue and GUI platform layer.

- [ ] Scan user-configured roots plus platform convention paths for CLAP bundles.
  Include canonical path, format, plugin ID/vendor/version, architecture and file
  fingerprint in cache entries; handle multiple plugins per library.
- [ ] Execute discovery in a separate helper process with timeout/crash quarantine.
  Test malformed binaries, symlinks, stale caches, incompatible CPU/OS and changed
  plugin versions. This isolates scanning; active DSP stays in the engine process.
- [ ] Native editor windows run on the OS main/UI thread, using CLAP GUI native
  APIs supported by the plugin/platform. Handle scale, resize, focus, hide/show,
  close/reopen and destruction before unloading. Provide a parameter editor if
  the plugin has no compatible GUI API.
- [ ] Test Linux X11/Wayland behavior explicitly; support Win32 and macOS Cocoa
  with platform runs. Webview-rendered HTML is not a substitute for native handles.
- [ ] Marshal plugin callbacks onto the correct thread; prevent editor callbacks
  from mutating audio-owned graph state directly. Test plugin removal with GUI open.

## Increment 6: web UI bridge and future Tauri desktop integration

Files: a native `EngineFactory` adapter near `WasmEngine.ts`, native control IPC,
and eventually an isolated Tauri shell. No shell is needed to validate the core.

- [ ] Version the transaction/control protocol, capability negotiation, sequence
  numbers, checksums, acknowledgements and reconnection/full-snapshot recovery.
  Transfer assets/state blobs on a non-realtime channel with explicit size limits.
- [ ] Use bounded SPSC queues at control/audio boundaries; preallocate native
  memory. UI/Tauri message handlers enqueue commands on a control thread. The
  audio callback never waits for the UI or deserializes JSON. Coalesce/drop stale
  telemetry with visible sequence/overflow counters; keep project edits ordered.
- [ ] Test UI stalls/disconnects, control backlog, sample delivery after deletion,
  repeated app startup/shutdown and checksum mismatch. Audio keeps rendering its
  last valid state while the UI is slow.
- [ ] Integrate native audio runtime with Tauri; OS plugin editors and callbacks
  stay on its main thread. Web UI controls native DSP without AudioWorklet for this
  backend. Browser deployment retains its WASM engine selection.
- [ ] Validate filesystem/state migrations from existing browser storage, asset
  availability offline, permissions, packaging, signing and OS updates separately.
  Validate webview behavior rather than assuming Chromium/WebKit parity.

## Increment 7: VST3 second

Files: separate VST3 format adapter and scanner support; reuse native graph,
event queues, routing, PDC, state storage and editor platform layer.

- [ ] Review current SDK licensing and distribution, then pin bindings/SDK.
- [ ] Implement module/factory lifecycle, component/controller connection,
  processing setup/activation, audio/event bus negotiation, parameter IDs and
  normalized values, process context, state streams and editor lifecycle.
- [ ] Map ordered host events to VST3 event lists and per-parameter sample-offset
  queues. Test controller gestures, restart notifications and latency/tail changes.
- [ ] Extend scan cache keys by format; test multiple class IDs in one module,
  unsupported interfaces, component/controller state round trips and missing plugin.
- [ ] Run the same offline/realtime, impulse/PDC, automation, lifecycle and GUI
  fixtures used for CLAP plus VST3-specific controller tests on each target OS.

Gate: do not start VST3 until CLAP lifecycle, state, scheduling and native graph
parity are reliable. No CLAP or VST3 hosting code belongs in Phase 1.

## Acceptance evidence

Each increment is a separate reviewable PR with native tests and a fresh WASM
build/parity run. Hardware callback latency, native plugin editors and production
plugin compatibility require desktop test machines; Linux cloud offline tests
cannot establish them. Optional future plugin subprocess isolation adds explicit
buffering/PDC cost and recovery semantics; do not make it the default graph path.
