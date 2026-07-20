# H-A29 dormant technology-canvas hook engineering

Status: `IMPLEMENTED_FAIL_CLOSED_STATIC_EXACT20_BOUND_LIVE_OBSERVATION_PENDING`.

`src/a729_technology_canvas.rs` is an isolated exact-a729 observer/carrier seam
for the UI09 technology canvas. It does not install detours and cannot emit a
product payload in this revision:

- `PRODUCTION_ACTIVATION` is `false`;
- `AUTHENTICATED_EXACT20_HOOK_CONTRACT` pins the receipt, both producer
  manifests, all 13 requested RVAs, and their 16-byte entry sentinels;
- `PERSISTED_EXACT20_PREFLIGHT_RECEIPT_SHA256` is `None`: each lease performed
  the broker readiness check, but no standalone same-session status artifact
  was persisted, so the receipt-dependent production gate stays false;
- `AUTHENTICATED_LIVE_PRODUCER_CONTRACT` is `None`; it must eventually bind the
  live observation, owner registry, installed patches, and trampolines;
- the source trait is crate-sealed;
- collection rejects before invoking the source while any live gate is absent;
- structural encode/decode helpers are private; the only public decoder checks
  production activation, persisted preflight, and the live contract;
- no collector scheduler, d71 fold, recorder fold, or replay consumer calls the
  module.

## Frozen boundaries

The seam names only retained, exact-a729 roots:

| Boundary | VA | RVA | Use |
|---|---:|---:|---|
| owner constructor | `0x140b52390` | `0x00b52390` | establishes the mode-6 owner slot at `+0xf8` and mode-7 slot at `+0x100` |
| typed controller constructor | `0x1413be7f0` | `0x013be7f0` | positively identifies `CCountryTechTreeView` and mode `+0x1aa4` |
| population commit | `0x1413c7410` | `0x013c7410` | complete folder enumeration/population boundary |
| folder materializer | `0x1413c1390` | `0x013c1390` | per-folder technology and connector construction |
| refresh commit | `0x1413c4890` | `0x013c4890` | refresh-side population/connector boundary |
| item rebind | `0x1413c8b70` | `0x013c8b70` | exact technology-definition rebinding |

Static authority is bound by
`UI09/H_A29_TECHNOLOGY_CANVAS_EXACT20_RECEIPT_20260720.json` (SHA-256
`6545f79563654c874cb078b3274d0f727237c093e6e845211f3f9672ee265f48`).
The root export is complete 6/6 on `shard-12`, manifest
`eab85630d79f7592bd1ae38f9ae5fb6102ddb5e2201cb3a1d6482a83cf5c74cb`;
the helper export is complete 7/7 on `shard-13`, manifest
`9837e5da6e0fb0c9cc4efdc49b1351b71a69810e5f94968e726da75679b2df5e`.
The contract covers all 13 requested producer roots, including non-detour
helpers that define identity, order, geometry, or scroll ownership.
The receipt also binds each job file, embedded request, manifest path/hash, and
each sentinel's instruction artifact/hash. It explicitly records that no
standalone same-session `ghidra_status` snapshot exists; reported ready/20
context is not treated as persisted promotion evidence.

Fresh decompile mining narrows—but does not erase—the live gap:

- `0x1413be010` constructs the legacy button-observer glue. The typed
  constructor installs code labels `0x1413c4550`, `0x1413c45b0`, and
  `0x1413c4720`. A follow-up function export rejected the address class at
  `0x1413c4550`; the failure is retained as negative evidence. A direct byte
  dump completed for all three labels, SHA-256
  `6f98be3d567c6f81a118f4bb95a2d79a055b9d1a8591152d42ef2b4eb2b790f7`.
  They are not represented as fabricated standalone function roots.
- `0x1413c10a0` walks authored path coordinates and dispatches directional
  connector segment builders. It does not itself produce a portable segment
  identity or final mixed draw ordinal.
- `0x1413c3040` resolves candidate `_tree` grid widgets by definition integer
  plus name and explicitly reports multiple candidates. This confirms that a
  grid name alone cannot be the portable slot identity.
- `0x1413c4480` clears/destroys the vector at controller `+0x1a40/+0x1a4c`.
  It is not evidence for clip, extent, pan, or scroll ownership.
- `0x1413bd410`, `0x1413bdd80`, and `0x1413bf850` are respectively a lookup-map
  insertion path, pooled item acquisition, and tab/widget append path. None
  closes complete exclusion dispositions or two-axis state.

Raw callback observations carry the callback RVA and require an exact
mode-6 owner-slot equality (`owner +0xf8 == controller`) before a bridge may
run. They require the original entry bytes saved before detour installation to
equal the exact20 sentinel, while installed patch and trampoline hashes must
equal the separate live contract. This avoids comparing a patched entry to its
original bytes. The wrapper must assert `PostReturn`, so a function-entry
snapshot cannot be mislabeled as committed. Portable owner identity is derived
from registry epoch, owner sequence, construction-record hash, and the
live-pinned registry manifest; it is not accepted as caller-supplied text.
Owner construction is an identity-only boundary and is explicitly unable to
emit a complete canvas. Only post-population and post-refresh boundaries are
eligible.

## Pointer-free carrier

The isolated `TCV1` carrier includes:

- exact executable, exact20 artifact receipt, persisted-preflight receipt,
  root/helper producer manifests, live observation receipt, live producer
  manifest, owner-registry manifest, and canonical content-manifest SHA-256
  values;
- a deterministic `source_record_id` bound to portable owner identity, game
  hour, callback sequence, boundary, static and live producer manifests,
  live-observation receipt, owner-registry manifest, and content manifest;
- complete source entries with contiguous source ordinals, drawable kind,
  full hierarchy path, folder identity, and included/excluded disposition;
- final mixed node/connector/overlay order;
- complete materialized drawable coverage, unique hierarchy-derived slot IDs,
  and exact rectangles;
- clip, content extent, viewport extent, explicit fixed-5 two-axis pan, and
  independent horizontal/vertical min/max/current/target values.

Validation requires source, final order, and materialized rectangles to be
exactly equal sets. A visible/materialized subset cannot pass. Raw pointers are
not representable in the carrier. Slot identity must equal the full authored
hierarchy path. The content manifest is recomputed from portable transition
identity and every canvas field; an arbitrary lower-hex hash cannot pass.

Static tests were added for round-trip preservation, incomplete coverage,
foreign/empty manifests, source-record transition binding, entry-versus-return
phase, saved-original/patch/trampoline comparison, complete 13-root contract
membership, registry-derived owner identity, content-manifest recomputation,
authenticated-decoder refusal, and compile-time activation refusal. Per
repository policy they were not run by this subagent.

## Remaining d71 gap

The existing d71 schema still cannot carry `TCV1` without losing required
proof:

1. no `source_record_id` or exact20/producer/content manifest binding;
2. no explicit two-axis pan;
3. materialized slots may be a strict subset of ordered items;
4. slot identity need not be a full authored hierarchy path.

`d71_projection_blockers()` exposes those four reasons mechanically. There is
deliberately no lossy projection function. Exact20 has supplied and pinned the
static helper exports and sentinels. The remaining gate is an authenticated
live fixture supplying complete dispositions, portable connector/overlay
identity, and the owning two-axis fields. The internal callback-label semantics
remain automated RE work, not a request for human interpretation. Only then may
root enable collection and revise d71, recorder fold, replay view, and the UI09
adapter atomically.
