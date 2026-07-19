# Event/compose master list

- Current surface: 36 gates; 31 direct passes, one known-negative control, four exact external-capture gates, zero post-port gaps, zero failures.
- Static-SRV closure bound: `frame_bundle.rs` `c6919d0a...`, verification `468035b7...`, trees oracle manifest `875047b5...`.
- Canonical FoW off graph PASS: terrain -> water -> rivers -> trees, final hash `1b554185e7b90769`.
- Canonical FoW on graph PASS: terrain -> water -> rivers -> trees, final hash `5a79c3503df3a519`.
- `live_trees_measure` is not run ad hoc: its preserved capture lacks water and rivers bundles, so the canonical graph is externally capture-gated.
- Other external gates: FINAL rivers pass-close payload, PAN2 RTV/DSV clear provenance, PAN-chunks unreadable preserved water SRV.
- `gate18_final` remains a pinned negative control; no final-frame parity claim exists for the four external gates.
- Remaining RE inputs outside these compose gates: M03 exact A48 and one synchronized O04 city draw.
- Manifest authority is allowlist-only; superseded logs and side results remain excluded.
