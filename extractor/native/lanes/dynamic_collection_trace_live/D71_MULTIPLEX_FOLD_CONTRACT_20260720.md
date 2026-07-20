# Domain 71 multiplex fold contract

Domain 71 is a transport reservation shared by two independent, magic-tagged
carriers:

- `DCT1` owns `_r2_dynamic_collection_trace_live`.
- `TCV1` owns `_r2_technology_canvas_live`.

The recorder classifies every record from its first four bytes before decoding
it. It retains separate per-hour `seen`, `invalid`, and value state for the two
carriers. Both sidecars may therefore be emitted in the same snapshot; emission
is not first-record-wins and does not use an `else if` branch.

A malformed or failure-status `DCT1` record nulls only the DCT1 sidecar for that
hour. A malformed `TCV1` record nulls only the TCV1 sidecar. Invalidity is sticky
within the hour, so a later valid record cannot hide an earlier torn record of
the same carrier. The state is reset at the next frame boundary.

Bytes with neither recognized magic are recorded as
`unrecognized_domain71_carrier` and poison both sidecars for that hour. Although
recognized DCT1 and TCV1 records are independently authenticated, the four-byte
magic is the multiplex envelope's only carrier assignment. An unrecognized or
torn discriminator cannot prove which logical carrier was damaged, so retaining
either same-hour value would turn ambiguity into an availability claim. Unknown
extensions must first be assigned a new recognized magic and decoder; they do
not receive permissive forward-compatibility. The unknown bytes are never
decoded, projected, or treated as evidence for either sidecar.

This routing does not alter the A28 heterogeneous materialized-slot template
contract. `CollectionTrace.row_template_id` remains the controller family/default
template, while every `MaterializedSlotTrace.row_template_id` retains the exact
slot template. TCV1 remains a lossless technology-canvas carrier and is never
projected into DCT1 rows.

Static recorder tests cover both asymmetric malformed cases and the unknown-
magic case. Production TCV1 admission remains gated by its authenticated live
producer contract; this multiplex correction does not activate that observer.
