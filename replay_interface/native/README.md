# replay_interface/native

Native replay workspace. Architecture of record:
`../documents/REPLAY_ARCHITECTURE.md`.

## Crates

| Crate | Role |
| --- | --- |
| `replay_core` | `.battleplan` ingest boundary + scrubbable `WorldTimeline` (the playback spine). |
| `replay_app` | The product surface: Win32 + D3D11 swapchain app, GPU map-mode recolor, camera, picking, timeline. |
| `map_viewer` | Map bundle tooling from the game install, CPU reference renderer, captured D3D11 pass validation/replay backend. |
| `terrain_capture` | D3D11 map-frame extraction/capture contract for map replay fidelity. |

## Checks

```powershell
cargo metadata --manifest-path replay_interface\native\Cargo.toml --no-deps --format-version 1
cargo test --manifest-path replay_interface\native\Cargo.toml --workspace --no-fail-fast
cargo fmt --manifest-path replay_interface\native\Cargo.toml --all --check
```

## Common commands

```powershell
# One-time inputs: generate the gitignored map bundle from the game install
cargo run --release --manifest-path replay_interface\native\Cargo.toml -p map_viewer -- bundle --game "<path-to-Hearts of Iron IV>"

# Build a synthetic demo recording from the bundle's real 1936 owners
cargo run --manifest-path replay_interface\native\Cargo.toml -p replay_core --bin make_demo_recording -- --bundle replay_interface\native\map_viewer\out\map_bundle --out replay_interface\native\map_viewer\out\synthetic_demo.battleplan

# Inspect any snapshot-flavor recording (container file or store dir)
cargo run --manifest-path replay_interface\native\Cargo.toml -p replay_core --bin replay_dump -- <recording> [--hour N]

# Run the replay app against the generated bundle
cargo run --release --manifest-path replay_interface\native\Cargo.toml -p replay_app -- [--bundle <dir>] [--recording <path.battleplan>] [--demo-scrub]

# Headless render of one timeline hour (no window; visual-QA seed)
cargo run --release --manifest-path replay_interface\native\Cargo.toml -p replay_app -- --recording <path.battleplan> --dump-frame out.bmp [--dump-hour <N>]

# Captured-pass terrain replay (parity track)
cargo run --manifest-path replay_interface\native\Cargo.toml -p map_viewer -- render-terrain-d3d11 --map-frame-bundle "<capture-dir-or-map_frame_bundle.json>" --out terrain_native.r16g16b16a16_float
```

Replay container docs live in `../documents/HOI4REPLAY_CONTAINER_SPEC.md`.
Extraction-seam requests live in `../documents/REPLAY_DATA_REQUIREMENTS.md`.
