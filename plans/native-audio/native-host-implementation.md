# Linux native host implementation

The user authorized continuation of the roadmap after reviewing Phase 1 and chose
Linux x86_64, CLAP first and VST3 second. Preserve the existing browser backend and
work on `feature/native-audio-engine-poc`; keep the PR in draft while integrating.

## First runnable milestone

Keep graph/transport/sequencing in the existing Rust engine. Add an opt-in native
facade and typed stock-device processing; no native path may dereference a `u32`
WASM address. Initially reject unsupported project devices/features explicitly.
Native global host contexts must be thread-local/scoped with a one-engine-per-thread
lease; the facade cannot be Send/Sync until resources are fully instance-owned.

Use maintained CLAP C bindings and library loading to host actual native binaries,
with fixture plugins exercising lifecycle, sample-offset events, parameters and
state. Then add VST3 using maintained generated SDK interfaces or an audited host
library. An offline-only library with callback mutexes must never be sold as a
hard realtime backend. Plugin libraries outlive all instances and callbacks.

The native executable exposes plugin listing, parameter inspection, offline
processing of WAV input or a test tone, state save/restore, MIDI note events,
and rendering supported `.od` projects. Graph DSP and hosted plugin DSP execute
in the same process. Validate outputs using fixture plugins and independently
decoded WAV data; preserve exact output frame counts and reject invalid arguments.

## Implementation tasks

- [x] Review Phase 1 and rerun its eight tests; draft the existing PR.
- [x] Add the native engine facade and typed production Vaporisateur processing; test real project
  loading, audio, independent engine lifetimes, transport and unsupported features.
- [x] Add `native-clap` with actual dynamically loaded fixture tests, MIDI/parameter
  events and state round trips. Reject unsupported bus configurations explicitly.
- [x] Add `native-vst3` after CLAP tests pass, with actual VST3 fixture loading,
  processing, parameter and state tests; document host-library runtime constraints.
- [x] Add `native-host` executable and generic streaming WAV writer. CLI tests
  must verify actual files, malformed input, overwritten paths and format failures.
- [x] Add process-isolated discovery, bounded MIDI/automation, parameter inspection
  and opaque state controls behind tested interfaces.
- [ ] Add hardware drivers and editor windows after a realtime-safe adapter is
  available; validate on a Linux desktop with audio hardware and a display.
- [x] Review the integrated diff, run native tests and production WASM builds/parity
  checks, then update documentation and draft PR with precise tested capabilities.

## Review focus

Partial ABI adaptation, plugin library unload before destruction, incorrect CLAP
or VST3 thread affinity, unsupported graph features silently rendering silence,
event/parameter data loss, error-induced output corruption and realtime claims
based only on offline tests. Tests and final reporting must address each.

## Execution ledger

- Phase 1 review: no blocking renderer defect; eight tests pass. Pointer truncation
  and no-op native dispatch confirmed. Typed sine entry point already exists.
- Ruling: adapt the device boundary with native typed objects, not by widening
  every WASM descriptor word; counts and float bit patterns are not addresses.
- Ruling: use thread-affine instance ownership while legacy host contexts are
  extracted; reject multiple live engines on one thread rather than alias globals.
- Finding: `SineDeviceBox` is not registered by the production project schema.
  The native facade therefore uses the production Vaporisateur synth, not an
  invented project format. TypeScript serializers generate deterministic `.od`
  and BoxGraph fixtures; the existing WASM linker renders the independent golden.
- Ruling: the CLI supports an optional master-output plugin in the same process
  as the graph. General saved graph insertion remains a separate next increment.
- Review-driven changes: bound signed codec counts/lengths; validate schemas,
  tuning and pointer relationships before publication; preserve plugin library
  lifetimes and continuous processing; reject stepped-parameter fractions; clean
  up incomplete CLI output on ordinary failures.
- Discovery probes run in their own Linux process groups with a five-second
  deadline, bounded output/aggregate metadata and crash/hang tests. Persistent
  scanning cache and crash isolation during DSP remain future increments.

- Independent-plugin validation: all four Dragonfly Reverb 3.2.10 effects render
  through both formats. Their VST3 lifecycle exposed a dependency field-drop-order
  defect; the complete published dependency is vendored with a narrow documented
  normal-teardown/rollback fix, retaining its original license and provenance.
