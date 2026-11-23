# 0308. Show force breakdowns only when their parts account for the total

The recorder's declared air and navy totals supply counts and rankings. A ranked window uses summary membership before reading any country's runs. A breakdown must exactly reconcile the recorded parts to the total before it can assign zero to an absent part. A total with an unknown split remains visible with an unknown breakdown. An unrecorded total is a gap, and a recorded zero with no parts is a known zero. Navy strength carries maximum hull points as its reference measure. Ship counts stand alone.

Part labels and grouping come from the recorded content. Plane variants group by archetype when resolved. A wing composition groups by its shared archetype or as a mix. Its signature is consulted at the last hour it is live in the window because the recorded signature belongs to one unbroken lifetime. Navy classes group by registered sub-unit or by the role indicated by the category mask. The reply keeps unresolved recorded identifiers distinct and groups neither missing data nor unsupported content under a guessed label.

[Force products](../../crates/dashboard-land/src/products/forces.rs)
