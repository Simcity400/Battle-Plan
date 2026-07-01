# Static Map Assets

This directory keeps legacy static map assets that are still consumed by the
recorder, save oracle, memory-reader research tools, and derived-layer tests.

New map generation and live map inspection are Rust-owned under
`replay_interface/native/map_viewer`. Do not add new Python or browser map tooling here.

The tracked JSON assets remain as compatibility data until the downstream
callers are migrated to the Rust bundle contract.

Generated PNGs under `replay_interface/map_pipeline/assets/` are ignored and should not be
committed.
