# replay_native

Native support crates for the Battle Plan recorder foundation.

The previous Rust replay app, map renderer, GUI runtime, widgets, and binding
crates were intentionally removed during the replay-surface reset. The remaining
workspace keeps only the pieces needed by the DLL connector and portable replay
container path.

## Crates

| Crate | Role |
| --- | --- |
| `collector` | In-process collector DLL, no-op attach ABI, pipe heartbeat, and verified root probe. |
| `map_viewer` | Rust-native HOI4 map bundle tooling and production D3D11 terrain replay backend. |
| `snapshot` | Snapshot-store and `.battleplan` reader/writer primitives. |
| `terrain_capture` | Rust-native D3D11 map-frame extraction contract for the production map renderer. |

## Checks

```powershell
cargo test --manifest-path replay_native/Cargo.toml --workspace --no-fail-fast
cargo fmt --manifest-path replay_native/Cargo.toml --all --check
```

Map checkpoint commands:

```powershell
cargo run --manifest-path replay_native/Cargo.toml -p map_viewer -- bundle --game "<path-to-Hearts of Iron IV>" --force
cargo run --manifest-path replay_native/Cargo.toml -p map_viewer -- render-terrain-d3d11 --map-frame-bundle "<capture-dir-or-map_frame_bundle.json>" --out terrain_native.r16g16b16a16_float
cargo run --manifest-path replay_native/Cargo.toml -p map_viewer -- validate-water-map-frame --map-frame-bundle "<capture-dir-or-map_frame_water_pass_bundle.json>"
cargo run --manifest-path replay_native/Cargo.toml -p map_viewer -- render-production-map-d3d11 --terrain-map-frame-bundle "<capture-dir-or-map_frame_bundle.json>" --water-map-frame-bundle "<capture-dir-or-map_frame_water_pass_bundle.json>" --rivers-map-frame-bundle "<capture-dir-or-map_frame_rivers_pass_bundle.json>" --borders-map-frame-bundle "<capture-dir-or-map_frame_borders_pass_bundle.json>" --trees-map-frame-bundle "<capture-dir-or-map_frame_trees_pass_bundle.json>" --camera-json production_camera.json --out map_native.raw
```

Production app code should load captured terrain with
`map_viewer::NativeD3d11TerrainBundle::load(...)`, optional captured water with
`map_viewer::NativeD3d11TerrainBundle::load_water(...)`, and optional captured
map-layer bundles with `load_rivers(...)`, `load_borders(...)`, and
`load_trees(...)`. `NativeD3d11TerrainRenderer::prepare_production_map_with_layers(...)`
prepares the app-owned terrain-first pass graph and replays ready dependent
passes into the same app-owned color/depth target with an app-owned camera. The
CLI is a verification wrapper over that backend, not the app integration point.

Reference pack/verify command:

```powershell
cargo run --manifest-path replay_native/Cargo.toml -p snapshot --bin pack_replay -- verify out.battleplan replay/snapshots campaign
```

Container spec: `../replay/HOI4REPLAY_CONTAINER_SPEC.md`.

## Guardrails

- The collector remains observational: no game-state writes or mutating game
  calls.
- Generated screenshots, output captures, game art, and local recordings stay
  ignored.
- Do not add personal paths or machine-specific install paths to tracked files.
