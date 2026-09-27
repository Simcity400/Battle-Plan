# dashboard

Everything a widget needs natively: opened recordings, data products, the host/viewer protocol and the data service that answers the frontend. No map pixels are drawn here and no live game memory is read.

## Layout

| Module | What it does |
|---|---|
| `source.rs` | One opened recording: replay bytes, metadata, resolved game content and each product's per-source cache. |
| `catalogue.rs` | Content facts every product shares, read once per source, folders and game language: the localisation (the same parse country naming uses, from `content::localisation`; text needing runtime scope is kept apart from a missing key) and interface sprites, plus the texture decoding, within a budget, and PNG encoding every image reply uses. |
| `metadata.rs` | Branch ranges, gaps and the first readable country catalogue; shared frame checks. |
| `products/` | One module per data product, registered once in `products/mod.rs`. |
| `service.rs` | The native data service: source leases, a loader pool and the bounded response pump. |
| `transport.rs` | Data commands, the generic product request and fragmented replies. |
| `protocol.rs` | Control messages between the Tauri host and the viewer process. |
| `map_settings.rs`, `config.rs`, `content.rs` | Map preferences, game folders and content verification. |
| `bin/dashboard-bridge.rs` | Development-only stdio adapter for browser UI work. |
| `bin/export-types.rs` | Generates the TypeScript contract. |

## Products

A product turns a source plus typed parameters into one compact reply covering a whole branch, so frontend playback is a local lookup and never a per-hour request. To add one:

1. Add `products/<name>.rs` implementing `Product` (a key, `Params`, `Reply`, optional `validate`, and `load`). Keep per-source caches in `source.state::<YourState>()`. Read localisation and sprites through `catalogue::catalogue`, never by parsing them again.
2. Register it in the `PRODUCTS` list in `products/mod.rs`.
3. Regenerate the contract and add the widget folder under `ui/src/widgets/`.

The service, transport, host and frontend route by key and never name a product. Replies are serialized with the frontend's lease identity so they cannot be attributed to another recording. Products read verified recorded content only; missing localisation or artwork stays null.

Current products: national focus order, research slot history, country flags, country styles (each country's engine-built name and recorded map colour at an hour), country groups (human players, recorded factions with their members, and war pairs, as runs of consecutive recorded hours), country balances (political power or command power: exact balance runs and net drops per country), country stability (displayed stability, war support and maximum command power computed from recorded inputs with the engine's arithmetic, plus an hour's formula terms, tree lines with their source identity or recorded text, and an explicit remainder) and land ownership. Their rules live beside their code and in the matching widget README.

## Service

`Service` owns sources by canonical path and revision, hands the frontend independently releasable leases, and runs product and open jobs on a small loader pool. Requests for the same product on the same source serialise on that product's cache; different products and sources run in parallel. Replies are bounded, fragmented and cancellable at every step. The map process shares the same sources through `open_map`.

## Types

Rust owns the generated frontend types. Run from the repository root:

```powershell
cargo run -p dashboard --bin export-types
cargo run -p dashboard --bin export-types -- --check
```

Generation updates `ui/src/generated/dashboard.ts`; do not hand-edit it. Follow with the checks in [AGENTS.md](../../AGENTS.md).

## Rules

Preserve recorded values exactly and keep missing, unavailable and zero distinct. Never bridge a gap or carry a value across missing hours. A changed recording revision invalidates every reply.
