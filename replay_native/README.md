# replay_native

Native Rust/wgpu replay application for inspecting HOI4 recordings.

The app is read-only: it opens snapshot stores or portable `.battleplan`
recordings and presents captured game state through game-faithful UI surfaces. It
does not write to HOI4, saves, or process memory.

## Role

This workspace owns the native app, rendering/runtime crates, view registry,
host/input plumbing, and replay playback. See
`../replay_ui/MASTER_PROGRESS.md` for current product status and acceptance
gates.

## Run

From the repository root:

```powershell
cargo run --manifest-path replay_native/Cargo.toml --release -p app --bin replay_app
```

Useful targeted checks:

```powershell
cargo check --manifest-path replay_native/Cargo.toml -p app --bin replay_app
cargo test --manifest-path replay_native/Cargo.toml -p app --no-fail-fast
cargo test --manifest-path replay_native/Cargo.toml -p render overlay --lib --no-fail-fast
cargo test --manifest-path replay_native/Cargo.toml -p assets --no-fail-fast
cargo fmt --manifest-path replay_native/Cargo.toml --all --check
```

## Crates

| Crate | Role |
| --- | --- |
| `snapshot` | Snapshot-store and `.battleplan` reader/writer primitives. |
| `assets` | Game asset, sprite, font, and DDS resolution. |
| `render` | wgpu map, terrain, weather, overlay, and mode rendering. |
| `gui_runtime` | Parsed `.gui` layout/runtime primitives. |
| `bind` | Data binding and no-fabrication display atoms. |
| `scrub` | Replay timeline/scrub support. |
| `focus_tree` | Focus tree parsing/layout/render support. |
| `combat_panel` | Combat model and panel rendering helpers. |
| `widgets` | Reusable UI widgets and interaction primitives. |
| `collector` | In-process collector DLL, no-op attach ABI, pipe heartbeat, and verified root probe. |
| `app` | Window host, view registry, native app binary, screenshot harness. |

## Portable Recordings

`.battleplan` files are ZIP containers with snapshots, identity metadata,
integrity manifests, and optional signature slots. The app-side reader shares the
same `Store` interface as directory snapshots, so portable recordings and local
stores open through the same path.

Spec: `../replay_ui/HOI4REPLAY_CONTAINER_SPEC.md`.

Reference pack/verify command:

```powershell
cargo run --manifest-path replay_native/Cargo.toml -p snapshot --bin pack_replay -- verify out.battleplan replay/snapshots campaign
```

## Guardrails

- Missing captured data must render unavailable/badged, never fabricated.
- Mutating HOI4 actions must be inert or disabled in the replay UI.
- Generated screenshots, output captures, game art, and local recordings stay
  ignored.
- Do not add personal paths or machine-specific install paths to tracked files.
