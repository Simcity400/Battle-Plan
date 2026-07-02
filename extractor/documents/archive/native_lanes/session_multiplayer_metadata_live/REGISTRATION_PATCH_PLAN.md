# session_multiplayer_metadata_live registration patch plan

Lane: `session_multiplayer_metadata_live`
Domain ID: `57`
Wire name: `session_multiplayer_metadata_live`

Shared files intentionally not edited by this lane packet:

- `extractor/native/collector/src/domain_registry.rs`
- `extractor/native/collector/Cargo.toml`
- `extractor/native/recorder_host/src/compact_domain_registry.rs`
- `extractor/native/recorder_host/Cargo.toml`
- `extractor/native/recorder_host/src/main.rs`

Required registration hooks:

1. Add the lane crate as a dependency in collector and recorder host:
   `session_multiplayer_metadata_live = { path = "../lanes/session_multiplayer_metadata_live" }`
   with the correct relative path from each crate.
2. Add domain descriptor `57 / session_multiplayer_metadata_live / session_multiplayer_metadata_live`
   to both shared registries as `ActiveIntegration` or `ValidatedSliceOpenGaps`, `schedulable = true`,
   `requires_data_record = true`.
3. Register the recorder data decoder by routing domain `57` `DOMAIN_DATA` payloads through
   `session_multiplayer_metadata_live::gate_payload` or equivalent summary adapter.
4. In the single injected DLL collector/coordinator, call
   `session_multiplayer_metadata_live::collector_module::collect_domain57` only after live session,
   command, message, and notification roots are validated for the current build.
5. Keep logs, saves, and Python probes as validation or fixture inputs only; production rows must come
   from `hoi4.exe -> injected DLL coordinator -> compact DOMAIN_DATA -> Rust recorder -> .battleplan`.

Current lane-local proof:

- Compact payload and stream gates are implemented in `src/lib.rs`.
- The proof binary emits domain-57 `DOMAIN_DATA`, `FRAME_END`, a recorder manifest, and a verified
  `.battleplan`.
- Live roots/hooks remain open; this packet is not a claim that the session metadata concept is complete.
