# Native audio engine: Phase 1 investigation

Investigated revision: `0c66d11f9` (2026-10-11). Paths below are relative to
the repository root. This investigation reads the actual implementation;
older WASM migration plans and comments can describe superseded behavior.

## Decision and scope

Native reuse is feasible for the existing Rust libraries. Running the complete
project engine natively with its current device interface is not a small change.
Add an isolated offline compatibility proof now; pursue a native graph with a web
UI for desktop plugin hosting. Keep browser/WASM builds as supported backends.

`crates/native-render` demonstrates the existing oscillator and actual summing
bus running outside a browser. It renders a fixed 1 kHz, gain 0.2, dual-mono tone
at 48 kHz in 128-frame quanta and streams a PCM16 WAV. It is not project playback,
does not instantiate `engine::Engine`, and does not exercise device registration,
transport, automation, or plugins. This narrower proof avoids a premature rewrite.

## Rust entry points and platform boundary

- `crates/engine/src/lib.rs`: private `Engine` owns the mirrored BoxGraph,
  transport and audio units. C exports include `init`, `apply_updates`, `bind`,
  `render`, transport controls, sample delivery, monitoring and device registration.
  `render` produces one stereo quantum; the planar output is exposed by pointer.
- `crates/engine/Cargo.toml`: `cdylib` only, not an importable Rust `rlib`.
  The crate is `no_std` outside tests; its talc global allocator is WASM-only.
  The non-test panic handler and singleton/global host context are unsuitable
  for embedding unchanged into a native `std` application with multiple engines.
- Native `call_device_process`, `call_device_init`, parameter hooks and MIDI-fx
  dispatch in that file are deliberately stubs. A successful native test compile
  does not establish working native device processing.
- `crates/abi/src/lib.rs`: `Ports`, descriptor arrays and many host interfaces use
  `u32` byte offsets converted to pointers. WASM function pointers are table slots.
  On 64-bit native targets these are neither valid pointers nor callable native
  function addresses. Simply changing the compilation target or casting `usize`
  to `u32` is unsafe. Some ABI host helpers return placeholders on native targets.
- `crates/stock-devices/*`: thin device `cdylib`s expose `state_size`, `kind`,
  `init`, `process` / `process_events` and optional lifecycle hooks. Much of the
  DSP itself is safe Rust; the exported descriptor and host callbacks remain
  platform-specific. Script, NAM and WebCLAP devices additionally call JS bridges.
- `crates/sine` is an earlier standalone generator with static global state and
  its own panic handler; it is not the current graph's sine device or a reusable
  native library. Use `dsp::osc::BandLimitedOscillator` instead.
- `crates/signalsmith` is now pure Rust; its C++ stretch oracle is a *dev* dependency.
  `stretch-lab` and `signalsmith-wasmbench` are already excluded research harnesses.

## Dynamic WASM loading and linking

`packages/studio/core-wasm/build-wasm.sh` builds `engine.wasm` on stable with SIMD,
imported memory and an imported indirect-function table. Devices are PIC side
modules built using nightly, PIC `core` via `-Zbuild-std=core`, hidden visibility
and immediate-abort panic handling. They have `dylink.0` sections, not ordinary
independent modules with private heaps. Builds explicitly select WASM packages;
adding a native workspace member does not include it in the WASM build.

`src/engine-modules.ts` supplies the module/box-type and composite registration
tables. `src/boot.ts::instantiateWasmEngine` creates a shared table, instantiates
the engine, initializes its sample rate and attaches JS script/NAM/WebCLAP bridges.
`src/device-linker.ts::linkDevice` then:

1. Reads each device's memory/table requirements from `dylink.0`.
2. Allocates data and a 256 KiB stack through `engine.device_alloc`, and grows
   the shared function table.
3. Supplies `env.memory`, table, `__memory_base`, `__table_base`, stack pointer,
   the authoritative engine host-import list and JS bridge imports.
4. Instantiates, applies relocations/constructors, installs exported functions
   into table slots and registers state size, device kind and lifecycle hooks.
5. Maps device IDs to box types and registers composite definitions as data.

The worklet, offline worker and Node test loader share this linker. Engine-to-stock
device calls use `call_indirect`, with audio and events in the same linear memory.
There is no OS dynamic library loading here. WebCLAP also exists today, but its
WASM/WASI bridge and GUI transport are not native CLAP binary hosting.

## AudioWorklet and realtime processing

`src/WasmEngine.ts` integrates with the studio's engine factory contract and loads
the AudioWorklet module. `src/processor.ts` owns engine memory and device instances.
`process()` calls `#render`: stage monitoring input, call `engine.render()`, copy
planar L/R into Web Audio output, update analysis/meters, publish state and drain
MIDI/clip/marker notifications. Default render quantum is fixed at 128 frames.
At 48 kHz that is 2.667 ms of audio, not a measured end-to-end latency.

Memory is currently **non-shared** between JS threads: the worklet owns it and
reads fresh `memory.buffer` after rendering because allocation can grow it.
SharedArrayBuffer is used for other streams/rings, not concurrent Rust heap access.
Some heap comments retain older shared-memory language; the build script and
current boot path establish the actual ownership.

`crates/engine-env/src/engine_context.rs` orders processors topologically;
`graph.rs` carries ordering only, while `SharedAudioBuffer` references carry audio.
Processors are `Rc<RefCell<...>>` and single-thread owned, which remains useful on
one native audio thread. Do not make them cross-thread by asserting `Send`/`Sync`.

The current worklet path includes allocations, reconciliation, possible heap
growth and JS bridge calls; it is not evidence of a hard realtime-safe native
callback. Native device lifecycle, graph edits, resource loads and state work need
explicit control-thread preparation and bounded publication at block boundaries.

`src/offline-worker.ts` already runs the same WASM engine without AudioWorklet,
using worker/RPC boot, pending resource handshakes and quantum rendering. This is
an excellent behavioral reference, but still needs a JS/WASM host.

## State, events, parameters and buffers

| Data | Existing route | Native implication |
| --- | --- | --- |
| Project | UI `BoxGraph` → `SyncSource` full dump then forward transactions → synchronous loopback → `sync/serialize-update-tasks.ts` → ordered RPC → WASM input scratch → `apply_updates` → Rust `decode_forward`/BoxGraph | Reuse schema/codecs and transaction/checksum semantics; define a versioned native control protocol rather than reimplementing project serialization. |
| Integrity | Ordered checksum requests compare 32-byte source and mirror checksums; divergence triggers engine restart | Preserve transaction order, rejection and resnapshot behavior. |
| Live notes | `EngineProcessor` RPC `noteSignal` → `note_signal_on/off/audition` | Define timestamped native ingress and its scheduling/overflow policy; the current UI RPC is not sample-accurate raw MIDI ingestion. |
| Sequenced notes | Rust note tracks/sequencers → pulse-domain event sources → device `host_pull_events` → `EventRecord` → instrument sub-block/sample offsets | Reuse pulse/sample conversion, note identity and lifecycle ordering; map to CLAP events rather than casting structs. |
| MIDI output | Rust queue → `midi_out_take` records → `WasmMidiDrain` → `MIDISender` SAB ring; MessagePort is a wake signal | Keep MIDI-output timestamps and scheduling, with native device I/O off the DSP callback where needed. |
| Parameters | Graph edits bind parameter paths and lifecycle hooks; automation/modulation produce timed updates through `host_update_parameters` and pulled parameter events | Reuse mappings and ordering, but translate IDs/ranges explicitly. Existing update-clock automation must not be described as arbitrary per-sample plugin automation. |
| Audio | `engine_env::AudioBuffer` planar `[f32; 128]` L/R; shared handles route graph audio; device descriptors point into the common WASM heap | Safe native borrows and native device callbacks should replace the offset layer. Driver block sizes need a preallocated adapter. |
| Samples/state | UI fetch/decode → async RPC response → `sample_allocate`/PCM copy/`sample_set_ready`; soundfonts analogous. Telemetry/state use fixed buffers, `SyncStream` and broadcasts | Decode/load off audio thread; swap prepared resources safely. Preserve stale-handle rejection and UI telemetry backpressure. |

Parameter ordering at equal pulse position in `engine/src/lib.rs` is note-off,
parameter update, note-on. CLAP translation must retain it and preserve stable
note IDs. The stock `EventRecord` is CLAP-shaped, not the CLAP C event ABI.

## Components reusable without source modifications

The PoC links `dsp` and `engine-env` (including the real `AudioBusProcessor`) on
Linux x86_64 unchanged; transitive `math`, `abi`, `value` and `boxgraph` compile.
The native workspace test run additionally checks library test behavior. Useful
libraries include `transport`, `bindings`, `processors`, `studio-boxes`, `voicing`,
`stretch` and `signalsmith`. Compile/test success must be distinguished from native
device-ABI execution, which this proof does not attempt. Check results and any
toolchain limitations are recorded in `validation.md`.

Best native abstractions are `Processor`, `ProcessInfo`/`Block`, `EngineContext`,
`Graph`/topological ordering, `AudioBuffer` and the parameter/note collections.
The TypeScript `EngineFactory`/`EngineProcessor` contract used by `WasmEngine` is
the UI integration seam. Preserve the existing wire format and browser adapter;
introduce native typed device dispatch behind the Rust host boundary. Extracting
an instance-owned engine core is preferable to duplicating its graph/mixer.

## Approach comparison

| Criterion | A: browser/WASM graph + external native host | B: native Rust graph + web UI |
| --- | --- | --- |
| Latency | Every plugin round trip crosses processes/scheduling domains, copies/rings and clock drift. At least the chosen bridge queue depth is added; browser output latency remains. Exact latency needs measurement. | Graph and plugin DSP share one callback/process; avoids per-plugin IPC. Device-driver latency and buffering still matter; 128-frame quantum needs adaptation. |
| Effort | Keeps existing graph, but needs native audio bridge, synchronization, failure handling and desktop transport. Arbitrary browser tabs cannot directly host native binaries. | Higher initial core/ABI extraction, driver and UI adapter effort; no need to rewrite reusable DSP or graph algorithms. |
| Compatibility | Browser DSP retained; native plugin architecture must match host OS/CPU. Split clocks and bridge editor/state handling complicate parity. | Native binaries still platform-specific; scripts, NAM and WebCLAP need adapters or explicit unsupported status. Browser remains its separate supported backend. |
| Reuse | Maximum immediate WASM runtime reuse; Rust native DSP reuse is limited. | High DSP, graph, transport, BoxGraph and event reuse once the boundary is extracted. |
| Maintenance | Two DSP schedulers plus realtime IPC protocol become permanent complexity. Useful transitional experiment, costly default architecture. | One desktop scheduler/graph and format adapters. Native/WASM host adapters need parity tests, but shared core limits divergence. |

Recommend B for desktop hosting, with graph and plugin DSP in one native process.
Use IPC only between UI/control and engine (or optional plugin isolation later),
not between every graph node. A remains a fallback experiment if core extraction
cost proves excessive; this offline proof measures no latency and proves neither
approach's full application behavior.

## Architectural risks and follow-up gates

1. Extract an instance-owned core/`rlib` and typed native device interface before
   claiming native project playback. Avoid global state and 32-bit pointer truncation.
2. Preserve WASM offset ABI exactly with differential graph/device fixtures.
3. Define allocation, event capacity, resource lifetime, edits and plugin thread
   affinity before connecting a realtime driver.
4. Keep scripts/NAM/WebCLAP support explicit; they do not become native automatically.
5. Evaluate licensing/distribution of plugin APIs and binaries before shipping a host.
6. Validate Windows/macOS native editors/drivers on their platforms; Linux offline
   rendering does not demonstrate desktop audio latency or editor support.

## AI-assisted contribution process

The user supplied purpose, phased scope, feature branch and PR authorization.
Read repository `AGENTS.md`, `CLAUDE.md`, README contribution guidance and current
Rust/TS sources. Wrote a bounded implementation plan in `/plans`, ran renderer
tests against an unimplemented contract, implemented only the offline adapter,
and requested a separate code review. No graph/DSP rewrite, production ABI change
or plugin hosting is included. Tool installation/logs live outside the repository.
Validation records failures and unavailable checks without treating compile-only
or Node tests as browser/hardware evidence.
