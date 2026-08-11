# Engine reverse-engineering evidence map

## Purpose

This file is a navigation index for map-rendering reverse-engineering evidence.
It is not a progress tracker, promotion receipt, backlog, or alternate product
status document. Current product status belongs to the map product-status
surface; normative behavior belongs to each packet law/contract; immutable
engine facts and hashes belong to packet-local evidence.

## Current exact-a729 authority

The active pinned binary is:

- `hoi4.exe 1.19.2.0.a729`;
- image base `0x140000000`; and
- SHA-256
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

Before a new Ghidra export, call `ghidra_status`. The current broker contract is
read-only/no-analysis with 20 ready shards, `shard-01` through `shard-20`.
Shard count is operational capacity, not semantic provenance. Evidence
admission checks the executing shard's binary identity, mode, job ID,
request/manifest hashes, and artifact hashes.

The `hoi4_re` facade owns shared symbol lookup, isolated new-build analysis,
registered memory observations, bounded call traces, constrained emulation,
and save/memory correlation. Start with `re_status`. It does not replace the
exact-a729 Ghidra authority and no trace or diff is automatic promotion
evidence.

## Packet evidence routing

| Packet | Mechanism | Canonical evidence or contract |
| --- | --- | --- |
| XMAP00 | non-UI semantic/carrier authority and effective-VFS census | `rendering_laws/XMAP00/` |
| XMAP01 | foundation feature coverage and adapter contract | `rendering_laws/XMAP01/FOUNDATION_PRODUCTION_CONTRACT.md` |
| XMAP02 | replace-path scope | `rendering_laws/XMAP02/replace_path_scope_a729.md` |
| XMAP03 | viewer camera and projection | `rendering_laws/XMAP03/PRODUCTION_EVIDENCE.md` |
| XMAP04 | terrain/map-copy/refraction | `rendering_laws/XMAP04/PRODUCTION_EVIDENCE.md` and `UNDERWATER_REFRACTION_REPAIR.md` |
| XMAP05 | political, gradient, FoW, and visibility resources | `rendering_laws/XMAP05/PRODUCTION_EVIDENCE.md` |
| XMAP06 | thin-border geometry, buckets, fade, and draw phases | `rendering_laws/XMAP06/PRODUCTION_EVIDENCE.md` |
| XMAP07 | water and snow/mud | `rendering_laws/XMAP07/PRODUCTION_EVIDENCE.md` |
| XMAP08 | rivers and straits | `rendering_laws/XMAP08/PRODUCTION_EVIDENCE.md` |
| XMAP09 | lighting, point lights, and light-depth shadow | `rendering_laws/XMAP09/PRODUCTION_EVIDENCE.md` |
| XMAP10 | aggregate SceneCB0, screen shadow, luminance, bloom, and RestoreScene | `rendering_laws/XMAP10/exact_a729_postfx_evidence_20260730.md` |
| XMAP11 | trees and vegetation | `rendering_laws/XMAP11/PRODUCTION_LAW.md` and packet JSON evidence |
| XMAP12 | cities, buildings, and fixed map objects | `rendering_laws/XMAP12/PRODUCTION_LAW.md` and `authority.json` |
| XMAP13 | units, counters, selection, and caster rows | `rendering_laws/XMAP13/PRODUCTION_CONTRACT.md` |
| XMAP14 | land/naval combat indicators and battle bubbles | `rendering_laws/XMAP14/` |
| XMAP15 | front lines and battle plans | `rendering_laws/XMAP15/PRODUCTION_EVIDENCE.md` |
| XMAP16 | rail and supply state | `rendering_laws/XMAP16/EXACT_A729_RAIL_SUPPLY_STATE_FIRST.md` |
| XMAP17 | country/state map names | `rendering_laws/XMAP17/PRODUCTION_EVIDENCE.md` |
| XMAP18 | map-mode interaction inputs | `rendering_laws/XMAP18/EXTRACTOR_DATA_REQUIREMENTS.md` |
| XMAP19 | weather | `rendering_laws/XMAP19/EXACT_A729_WEATHER_PRODUCTION.md` |
| XMAP20 | entity effects | `rendering_laws/XMAP20/PRODUCTION_EVIDENCE.md` |
| XMAP21 | map furniture | `rendering_laws/XMAP21/EXACT_A729_EVIDENCE.md` |
| XMAP22 | cross-packet composition and presentation | `rendering_laws/XMAP22/COMPOSITION_SEQUENCE_CONTRACT.md` and packet evidence |

Paths in the table are relative to
`replay_interface/re_evidence/rendering_laws/`.

## Evidence rules

An exact engine claim records:

1. executable identity and image base;
2. job and manifest identity;
3. the relevant function/instruction/data artifact hash;
4. the precise semantic conclusion supported by that artifact; and
5. any boundary the artifact does not prove.

Game-file claims name the effective-VFS source and distinguish authored content
from runtime semantics. Capture claims identify the exact retained object and
remain comparison evidence unless a packet contract explicitly admits that
carrier. Recorder camera, selected-Present pixels, live pointers, and capture
resource ordinals are never promoted by implication.

Human-friendly names are not invented for unproved engine predicates. A packet
may authenticate a raw branch result, ordinal, or digest while leaving its
friendly name unspecified. Missing evidence produces a typed unavailable or
hard failure according to the packet contract, never a tuned substitute.

## Source ownership

The normal product implementation is under:

- `replay_interface/native/map_viewer/src/production_features/` for packet
  preparation;
- packet-owned native handoff modules in `map_viewer` for D3D11 resources and
  draw receipts;
- `replay_interface/native/replay_app/src/app.rs` for the shared composition
  session and window/headless execution; and
- `replay_interface/native/replay_core/src/world.rs` for typed replay-world
  decoding and domain access.

Legacy capture replayers, CPU visualizers, diagnostic dump binaries, and
retained experiment modules are evidence/debug surfaces. Their existence does
not create a second product route or weaken the packet-local fail-closed laws.
