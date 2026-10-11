# Production project fixtures

Run from the repository root after building the browser WASM artifacts and installing workspace dependencies:

```sh
native_fixture_dir=$(mktemp -d)
node_modules/.bin/esbuild crates/engine/tests/fixtures/generate.ts --bundle --platform=node --format=esm --outfile="$native_fixture_dir/generate.mjs"
node "$native_fixture_dir/generate.mjs"
```

The default WASM paths are `packages/studio/core-wasm/dist/wasm/engine.wasm` and `packages/studio/core-wasm/dist/wasm/plugins/device_vaporisateur.wasm`. Set `NATIVE_ENGINE_WASM` and `NATIVE_VAP_WASM` to override them with equivalent production engine and dynamically linked device builds. The device must use the browser's position-independent WASM build, rather than a standalone WASM module.

The generator uses the production `ProjectSkeleton`, registered box classes, graph serializers, forward transaction serializer, device linker, and WASM renderer. Sequential UUIDs and a fixed project creation date make regeneration deterministic. The project contains the actual Vaporisateur instrument, one MIDI note, its track and region, and the production primary bus/output routing; the output maximizer is disabled because the native subset rejects unsupported devices.

`vaporisateur.snapshot` is the serialized graph. `vaporisateur.od` is the production `OPEN` version 2 project envelope. The 32-byte checksum files come from the TypeScript graph. The initial-update file is its production forward transaction dump; gain-update changes the instrument audio unit's gain to -6.0206 dB.

Each `.wasm.f32` file contains 20 consecutive 128-frame stereo render quanta at 48 kHz. Samples are little-endian float32, planar within each quantum: 128 left samples followed by 128 right samples. The gain golden is rendered after applying the live gain transaction. Both sequences call stop/reset and then play before rendering, matching the existing WASM transport reset sequence and the native parity tests.

The native tests require graph checksum equality, audible finite output, transport advancement, silence after stop, and a maximum absolute native/WASM sample difference below `1e-5`. They also verify malformed graph counts, missing schema fields, invalid pointer roles, invalid tuning, and unsupported devices are rejected before replacing a live graph.
