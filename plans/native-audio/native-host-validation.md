# Linux native continuation validation

Validated on 2026-10-11 in Linux x86_64 Codex Cloud with Rust 1.99 and Node 24,
on `feature/native-audio-engine-poc`. Scope: offline production graph subset,
CLAP first and VST3 second; no hardware or desktop claims.

## Passing checks

| Check | Result |
| --- | --- |
| `cargo test --workspace --exclude studio-boxes --offline --locked` | 976 passed, 0 failed, 7 ignored |
| Native engine with `--features native` | 220 unit tests and 7 integration tests passed |
| Four native crates focused suite | 39 passed; optional third-party test also exercised separately |
| `cargo build -p native-host --release --offline --locked` | Passed |
| Native crate formatting | Passed |
| Scoped Clippy, all targets, `--no-deps -- -D warnings` | Passed |
| Production `npm run build:wasm` | Engine, analysis and all stock PIC devices built |
| `npm run build:bundles` | Worklet and offline-worker bundles built |
| Five relevant Vitest files | 6 tests passed |
| Fixture regeneration against fresh WASM | All binary fixtures byte-identical |
| Release CLI smoke test | 6 commands passed against real loaded plugin binaries |

Selected Vitest files: `bundle`, `sync`, `sync-update-then-delete-in-batch`,
`strip-volume-automation`, `automation-edit-no-survivor-push`. The earlier full
Vitest run and its environment-dependent failures remain recorded in
[Phase 1 validation](validation.md); the expensive full suite was not rerun here.
`wasm-opt` is absent, so the existing script ships its permitted unoptimized modules.

Production TypeScript serializers generate the graph, OPEN project and forward
transaction fixtures; the actual WASM linker loads Vaporisateur for the golden.
Baseline and live gain-edit audio matched exactly: maximum sample difference
**0**, with a regression tolerance of `1e-5` over 5,120 float samples per fixture.
Checksums match. Tests cover independent thread contexts, transport/stop, malformed
counts/schemas/tuning/pointer roles and rejection before live graph publication.

CLAP/VST3 tests load actual fixture shared libraries using maintained bindings.
They cover lifecycle, unload/reload, continuous blocks, offset MIDI/parameters,
state round trips and invalid inputs. CLI tests inspect WAV samples/tails, restore
state, process ZIP/project audio through a master plugin, scan through the actual
helper, reject malformed input/stepped values, prevent overwrites and remove
ordinary-error partial output. Scanner tests cover crashes, hangs, inherited
output, malformed metadata and individual/aggregate limits.

Release smoke WAVs are stereo PCM16/48 kHz: 48,000 frames through each gain plugin
and 2,560 native project frames through VST3. CLAP and VST3 0.5-gain output WAVs
are byte-identical, SHA-256
`9d9b82aed46d3c659690cc851dda1137943d1846a42f5aea295f7372e7e2ff9c`.
The original oscillator WAV retains Phase 1's SHA-256
`b378d20122e51a5ced19784f12f4194fa712e4988e9cd7533218ead832342d24`.

Official Dragonfly Reverb 3.2.10 Linux x86_64 Early Reflections, Hall, Plate and
Room effects were also loaded and rendered in both CLAP and VST3. Each produced
48,000 frames at 48 kHz, with byte-identical WAV output between formats. The final
patched release executable reran all four VST3 renders with empty stderr. These are
independent released plugins, not the repository fixtures; binaries remain
outside the repository. All four VST3 effects additionally passed three
warning-free load/process/drop cycles after the dependency fix.

Dragonfly exposed an inherited VST3 dependency release-order defect. The narrow
[vendored patch](../../crates/native-vst3/vendor/README.md) preserves the complete
MIT-licensed published package and changes only field order in one source file,
so child interfaces release before their owning component/controller during
normal teardown and failed-load cleanup. The actual-plugin regression reproduced
warnings before the fix and passed afterward. Replace the patch when upstream
ships the equivalent fix.

## Existing failures and environment setup

`cargo test -p studio-boxes` still fails the pre-existing
`all_boxes_round_trip_byte_identical` comparison: 2 pass, 1 fails. Phase 1
reproduced it on the unchanged base. It is excluded from the passing command
above and is not treated as passing.

Core-WASM typecheck fails with 12 diagnostics in unchanged tests and DOM types
(`AudioPlaybackStats`, capture latency). A baseline source archive with the same
installed dependencies reproduced the identical 12 diagnostics. No production
TypeScript changed.

Unrestricted dependency linting hits pre-existing ABI interior-mutable constant
lints under Rust 1.99. Strict Clippy for the native crates uses `--no-deps`.

Cloud has `libxcb.so.1` but lacks its development linker symlink. Builds use a
workspace-only `libxcb.so` symlink and `RUSTFLAGS=-L native=...`; no system files
changed. Normal Linux setup uses `libxcb1-dev` as documented in the CLI README.
Signalsmith's existing C++ oracle needs
`BINDGEN_EXTRA_CLANG_ARGS=-isystem /usr/lib/gcc/x86_64-linux-gnu/14/include` here.
Rust 1.85 is the declared host minimum; this run used 1.99, not the minimum toolchain.

## Remaining limits

Cloud has no audio device or display. Hardware latency, editor windows, audible
playback and Tauri integration are unverified and unimplemented. Compatibility
beyond the tested Dragonfly effects and fixture binaries, including commercial
plugins, remains unverified.

The graph supports a production subset, not arbitrary projects. The optional
master-output plugin is not a saved graph device. Plugin transport, tails,
sidechains and PDC are absent. State blobs need plugin ID/version stored alongside.
Scanning is isolated; DSP is not. Plugin crashes can terminate rendering and leave
an incomplete file.

Thread-affine graph and offline adapters are not realtime backends. VST3's
maintained host locks and allocates during processing. Full context ownership,
realtime/control separation, stock-device coverage, saved graph integration,
drivers and editors remain next stages.
