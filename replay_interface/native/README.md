# replay_interface/native

Native map and replay-interface workspace.

This folder owns visual replay work that consumes game-backed map captures and
eventually `.battleplan` data produced by the extractor.

## Crates

| Crate | Role |
| --- | --- |
| `map_viewer` | Rust-native HOI4 map bundle tooling and D3D11 terrain replay backend. |
| `terrain_capture` | D3D11 map-frame extraction/capture contract for map replay fidelity. |

## Checks

```powershell
cargo metadata --manifest-path replay_interface\native\Cargo.toml --no-deps --format-version 1
cargo test --manifest-path replay_interface\native\Cargo.toml --workspace --no-fail-fast
cargo fmt --manifest-path replay_interface\native\Cargo.toml --all --check
```

## Map Commands

```powershell
cargo run --manifest-path replay_interface\native\Cargo.toml -p map_viewer -- bundle --game "<path-to-Hearts of Iron IV>" --force
cargo run --manifest-path replay_interface\native\Cargo.toml -p map_viewer -- render-terrain-d3d11 --map-frame-bundle "<capture-dir-or-map_frame_bundle.json>" --out terrain_native.r16g16b16a16_float
cargo run --manifest-path replay_interface\native\Cargo.toml -p map_viewer -- validate-water-map-frame --map-frame-bundle "<capture-dir-or-map_frame_water_pass_bundle.json>"
```

Replay container docs live in `../documents/HOI4REPLAY_CONTAINER_SPEC.md`.
