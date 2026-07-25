# A27 live designer-view context (domain 78)

This lane records the exact a729 equipment-designer context after the engine's
visibility-triggered refresh. It is deliberately separate from domain 59's
static `templates_equipment_catalog_live` artifact.

Production flow:

1. Signature-gate and detour `0x1417797A0`.
2. Call the original engine callback first.
3. Copy mode/family, current-country identity, raw toggle bytes, the selected
   country-tag vector, and the final used row-pool order.
4. Re-gate every row and variant by exact a729 vtable/type/stable ID.
5. Serialize only numeric country/design identities. No view, handler, row,
   variant, country, vtable, or module address enters DCX1.
6. Fold DCX1 into `_r2_design_catalog_context_live`; replay-core and game-UI
   reject absent, malformed, unauthenticated, duplicate, or unknown identities.

An installed hook with no complete transition emits a typed
source-unavailable frame, not an inactive or measured-empty view. A hidden
transition is positive evidence and encodes `active=false` with an empty final
order.

Canonical RE provenance and the promotion disposition are in
`../../../documents/domains/d78_design_catalog_context_live.md`.
The same address/job/artifact/hash inventory is frozen as machine-readable
evidence in `proofs/a27_designer_view_context/manifest.json`.
