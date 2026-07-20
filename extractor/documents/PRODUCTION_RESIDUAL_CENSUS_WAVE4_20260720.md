# Wave 4 production residual census

Date: 2026-07-20
Baseline: commit `832a2567`
Scope: extractor producer -> compact D43 carrier -> recorder authority/fold -> portable snapshot -> typed replay -> XUI15 product admission/render composition
Policy: exact-or-fail; no inferred booleans, no visual tuning, and no conversion of an absent legacy field into `false`.

## Selected residual

The highest-value non-human residual not owned by another Wave 4 lane was the
two exact classic-resource-route status bytes already frozen in the X15 RE
contract:

- `u8[CResourceDeliveryRoute+0xE8]`: no trade influence;
- `u8[CResourceDeliveryRoute+0xE9]`: no resources.

Before this change, the production collector read the same route object for
identity, resource, amounts, factories, convoys, route kind, and efficiency,
but serialized zero into the MTRD row's `+0x1C` reserved word. The recorder
therefore could not distinguish measured false from absent, and the existing
typed replay/UI leaves correctly remained unavailable. This was an integration
loss, not an unresolved semantic or human-observation problem.

## End-to-end disposition

| Boundary | Before | Wave 4 disposition |
|---|---|---|
| Extractor | Exact offsets documented, bytes not read | Stable reads admit only raw `0` or `1`; any other byte rejects the route snapshot. |
| Compact carrier | MTRD v1 row `+0x1C` reserved zero | MTRD v2 stores a two-bit status mask in the same 96-byte row; no size or pointer-bearing field is added. |
| Diagnostic lane | Only v1 shape accepted | v1 and v2 are accepted; v2 rejects unknown bits and reciprocal halves must agree; v1 reports `None`. |
| Recorder authority | Only v1 header accepted | Header admission accepts exactly v1 or v2. |
| Recorder fold | Reserved word required zero | v2 validates the known mask, preserves `Option<u32>`, and rejects half disagreement; v1 requires zero and preserves absence. |
| Portable snapshot | Both JSON leaves omitted | v2 emits measured `no_trade_influence` and `no_resources`; v1 omits both. |
| Typed replay | `Leaf<bool>` support already present | Existing decoder maps v2 booleans to `Measured`; omitted v1 leaves stay `Absent`. |
| Product UI | Optional route status already represented in `TradeDealCard` | Existing XUI15 admission receives exact booleans without deriving them from delivery, convoy, or route-kind state. The shipped row has no independent authored status widget, so no synthetic draw is added. |

The reciprocal route halves share one source object and must carry identical
status bits. A disagreement rejects the entire MTRD trailer. The current row
size stays 96 bytes, and the trailer version is the only compatibility switch.

## Exact static test additions

- Collector packing asserts MTRD version 2 and the exact status-mask word.
- `market_trade_live` asserts v2 status preservation, reciprocal agreement,
  and v1 absence rather than false.
- `world_fold` asserts v2 portable booleans and v1 key omission.
- The XUI15 promotion gate now binds the collector, diagnostic decoder,
  recorder authority, fold, and fold tests in addition to the existing typed
  replay and UI consumers.

Root-owned centralized validation is complete. Final/All and Release/All passed
with exit zero; the release receipt
`.cargo-target/release/hoi4-production-prebuilt-artifacts.json` has SHA-256
`f5f78fd251e6d3720ff69d5fd3e7904c452ae14fd792f5bbb0837e01ca388d0a`.
The combined Python battery passed 471 with 44 skipped, and the production
capture-script battery passed 79/79. The lane itself did not run Cargo, builds,
tests, verifiers, compilers, formatters, or Ghidra; validation remained
root-owned.

## Other fresh census results

- X15-P1 through X15-P5 were stale in the active ledger: MTRF v2, MTRR, MTRH,
  MTRD, complete country witnesses, recorder projection, typed replay, and the
  XUI15 screen already existed. Their active status is corrected to production
  integrated; only the route-status transport described above was a real open
  X15 producer leaf.
- B1 `MTS1` reaches recorder portable JSON but does not yet have a dedicated
  typed market-stockpile product view. It is a legitimate later product seam,
  not a blocker for the classic resource-trade XUI15 surface selected here.
- The remaining high-priority current residuals found in the active ledgers
  are owned by the concurrent CID1, A18, A29, A33, A34, A37, A39, A40, C07,
  CAP6, G2, restore, and weather lanes. They were intentionally not edited.
- No human input is required for MTRD v2. The exact offsets, byte semantics,
  reciprocal route identity, and replay leaves were all already authenticated.
