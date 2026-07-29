# D55 non-suspending hook feasibility

## Scope and product boundary

This note covers the ten exact-a729 D55 observation targets in
`land_combat_detail_events_live`: the three terminal-event targets and the
seven X2 width, loss, and panel targets.

The production recorder may not pause the game, suspend target threads, drive
the client, or depend on launch-time process suspension. MinHook is therefore
not a promotable primitive: its live enable, disable, and removal paths suspend
target threads. An aligned data compare/exchange over executable bytes is also
not admitted merely because the write is atomic. The exact instruction-fetch
and cross-core synchronization contract must be proven independently.

## Pinned evidence

All evidence below is from the read-only exact-a729 broker:

- Binary: `hoi4.exe 1.19.2.0.a729`
- Image base: `0x140000000`
- SHA-256:
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`
- Function/xref job:
  `20260728T233708Z-d55-nonsuspending-hook-site-feasibility-387689ce`
- Function/xref manifest SHA-256:
  `2f7d4c55a8282dabff5c48f3f834498d7c0102f04215fb7b67c00dfd9fe73897`
- Independent X2 all-data-xref review job:
  `20260728T235243Z-d55-x2-adversarial-all-data-xrefs-1edca5a4`
- Independent X2 all-data-xref artifact SHA-256:
  `8ebaf4a9e49293afb6172d41a24e4f9ed39e6ca6d1431361882b766f9fbb355c`
- X2 pointer-array classification job:
  `20260728T234002Z-d55-x2-pointer-ref-classification-5712cf66`
- X2 pointer-array artifact SHA-256:
  `3ffc0820812523787e4a27a2f9e103e4e9b94bcfc8cba35b82c31983a4476e3b`
- X2 direct-caller job:
  `20260728T234401Z-d55-x2-direct-caller-dispatch-feasibilit-8e9b6e2e`
- X2 direct-caller manifest SHA-256:
  `1b23367c931d403ac3ba97ca0a490ddaa736dfb4a10bdbe1d6f1c9754170e404`
- X2 caller-slot classification job:
  `20260728T234441Z-d55-x2-caller-dispatch-slot-classificati-93d6a9a3`
- X2 caller-slot artifact SHA-256:
  `868e7055b8eed8f152928c4c73732166280a38110ff489d21dbb604e7e4e9b36`
- Terminal upstream-root job:
  `20260728T234702Z-d55-terminal-upstream-engine-indirection-9a9875bf`
- Terminal upstream-root manifest SHA-256:
  `9f48cd8e9b4ecb298098961d6ec9fc4ff29fd51803a0273606169284cdef37f0`
- Terminal upstream-slot classification job:
  `20260728T234950Z-d55-upstream-root-rdata-slot-classificat-d3a5c4c7`
- Terminal upstream-slot artifact SHA-256:
  `b0d8ffa660d2521ed560010eb58852db3f7b1ea1a9f91c47c9ace82377456153`

The broker was rechecked after the Codex restart and reported read-only,
no-analysis mode, image base `0x140000000`, the exact SHA-256 above, and all
twenty configured shards ready. Shard count is operational capacity, not
semantic provenance.

## Exact X2 classification

| Exact target | Required semantics | Exact incoming topology | Non-suspending disposition |
|---|---|---|---|
| `0x140C7BFC0` active-damage loss writer | Before/after loss delta around the complete writer | Three direct `CALL rel32` sites; the other reference is `.pdata` | No engine-owned pointer slot at the target |
| `0x141299850` participant lifecycle | Before/after loss delta around lifecycle mutation | Pointer slots `0x1429914C8` and `0x1429A5920`; the other reference is `.pdata` | Exact naturally aligned engine-owned pointer slots exist |
| `0x141299610` width producer | Producer boundary, side identity, ordered wrapper group, final sum | Seven direct `CALL rel32` sites; the other reference is `.pdata` | No engine-owned pointer slot at the target |
| `0x140C6E270` division-width leaf | Ordered per-wrapper exact width result | Pointer slot `0x142943C40`; the other reference is `.pdata` | Exact naturally aligned engine-owned pointer slot exists |
| `0x1415C9860` panel side materializer | Side-scoped boundary around leader and modifier materialization | Two direct `CALL rel32` sites; the other reference is `.pdata` | No engine-owned pointer slot at the target |
| `0x1415BBF00` panel modifier append | Exact displayed modifier membership and order | Ten direct `CALL rel32` sites; remaining references are `.pdata` or unwind metadata | No engine-owned pointer slot at the target |
| `0x14129E190` panel leader resolver | Game-selected leader | Pointer slots `0x1429914A8` and `0x1429A5900`; remaining references are `.pdata` or unwind metadata | Exact naturally aligned engine-owned pointer slots exist |

The apparent `0x143...` data references are PE64 runtime-function entries in
`.pdata`. The `0x142F...` references called out above are unwind metadata. They
are not dispatch indirection and must not be patched or reported as hook slots.

Several *callers* of the four direct-only targets have vtable entries. That
does not preserve the current carrier contract: an outer-caller detour does not
receive the exact nested target arguments, and the recovered evidence does not
prove that every old caller has quiesced before its body or nested callsite is
changed. Caller vtables therefore do not convert the direct-only target roster
into a complete atomic-slot roster.

## Terminal targets

The three terminal targets likewise have no proven engine-owned pointer
indirection at their exact entries. Their apparent data references classify as
`.pdata` or unwind metadata, while their semantic incoming edges are direct
calls. Entry padding is not a Microsoft hotpatch contract: the entries begin
with real prologue instructions and no recovered evidence establishes a
two-byte hotpatch prefix or a target-thread quiescent installation boundary.

The bounded upstream search found two real engine-owned pointer-array slots:

- normal root `0x140C78A90` at aligned slot `0x142943B60`, and
- overrun root `0x1413D93F0` at aligned slot `0x1429A5750`.

They are not semantic substitutes for the terminal hooks. The normal root
passes one argument to an intermediate function, which conditionally derives
the five arguments consumed by writer `0x140CCC7F0`. The overrun root likewise
passes one argument to an intermediate function that mutates state before
deriving the three arguments consumed by writer `0x140CCC100`. A root wrapper
cannot observe whether the conditional writer ran or recover the exact
invocation arguments at the writer boundary.

Allocator `0x140CC9F10` is independently decisive. Each writer passes a
writer-local stack record to the allocator and consumes the returned index.
Neither upstream root wrapper can observe that exact stack pointer and returned
index. Inferring them from the global pool after the root returns is not the
same evidence boundary and has no exact concurrency proof.

## Promotion verdict

The present ten-site MinHook implementation is not promotable under the
zero-client-control boundary.

Three X2 targets have a viable atomic pointer-slot primitive in isolation, but
partial conversion is not a production solution. Four mandatory X2 semantics
and all three terminal semantics still lack either:

1. complete engine-owned pointer indirection that preserves the exact target
   arguments and boundary, or
2. an independently proven natural quiescent boundary covering every thread
   that can fetch the modified executable bytes.

No such complete boundary exists in the recovered static evidence. A low-level
collector/extractor harness can validate queueing, throughput, losslessness,
sink acknowledgement, and teardown after a safe primitive exists; it cannot
prove the game cross-core instruction-fetch quiescence. Live game/UI driving is
neither required nor permitted to manufacture that proof.

Until one of the two missing proofs is supplied, production must not install,
enable, or advertise the ten-site MinHook roster. This is an exact hook-
primitive blocker, not a reason to pause, slow, or otherwise control the game.
