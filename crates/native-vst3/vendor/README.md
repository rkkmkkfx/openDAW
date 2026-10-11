# VST3 host release-order patch

This directory contains the complete published `vst3-host` **0.10.0** crate from
crates.io, retaining its original normalized manifest, MIT license, documentation
and `.cargo_vcs_info.json` provenance. Upstream repository:
https://github.com/HelgeSverre/rust-vst3-host . The downloaded registry package is
the reference; no dependency API or generated VST3 ABI is changed.

The workspace uses `[patch.crates-io]` to select this local source. The sole
changed upstream source file is `src/internal/plugin_impl.rs`: move the `controller`
and `component` fields below all processor/view/connection references and before
`_module`, with a comment explaining the order. Rust destroys struct fields in
declaration order after the destructor runs. Existing `Drop` still stops DSP,
disconnects the interfaces and terminates the component/controller first.
Remaining references then release child interfaces, controller, component,
module/library and host application context in that order. The same field-order
fix is applied to `InitializedComponent`, the early-load rollback guard: its
connection references release before controller/component references.

`release-order.patch` records the exact source difference. Compare the vendored
file with the registry's `vst3-host-0.10.0/src/internal/plugin_impl.rs` using
`diff -u`, or apply this patch to a pristine published package to reproduce it.
No source-generation or build-script patching is used.

The released dependency previously released `IComponent` before its separately
referenced processor and connection points. Official Dragonfly Reverb 3.2.10
VST3 plugins emit DPF warnings that the parent is being deleted while child
interfaces still have reference count 1. DPF defers parent deletion until factory
cleanup, avoiding an immediate use-after-free in those plugins, but other plugin
implementations may handle the order differently. This is a lifetime compatibility
fix, not suppression of stderr or leaking plugin instances.

The current upstream main revision `7bece4e39cff2925a0a33b40092f2c2dcaaea9ac`
was inspected and retains the original field order. Remove this vendor patch
when a maintained release provides equivalent child-before-parent release order,
after rerunning fixture tests and third-party lifecycle smoke tests.

Dragonfly 3.2.10's DPF submodule is
`dc6557a3c935ae4bb68f89d8bc698a805fff788e`; its
`distrho/src/DistrhoPluginVST3.cpp` contains the warning/deferred-cleanup logic.
Third-party plugin binaries remain outside this repository.
