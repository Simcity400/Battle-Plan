# dashboard-air-superiority

The air superiority product: each strategic region's friendly and enemy air
scores, as moment windows and daily timelines. Behaviour is documented beside
the code in [products/air_superiority.rs](src/products/air_superiority.rs) and
in the widget guides ([air superiority
region](../../ui/src/widgets/air-superiority-region/README.md), [air superiority
timeline](../../ui/src/widgets/air-superiority-timeline/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md),
laid out like
[dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the module
keeps its dashboard path and names what it uses at the crates that own it
(dashboard-base, dashboard-text and dashboard-core). The
[dashboard](../dashboard/README.md) re-exports it at its old
`dashboard::products::` path, registers it, and prepares and queries it in its
pipeline.

`cargo test -p dashboard-air-superiority` builds and runs only this crate's
tests.

Choices and daily timelines answer before complete preparation. Published daily
values supply the prefix; one keyframe per later block supplies approximate
cells, the side identities and the most-active row selection. Samples keep their
actual supplying hours in `sampleHours`, independently of the days they stand
for. A daily cell crossing the prefix boundary is wholly approximate. Unread
keyframes stay pending and recorded gaps retain their own shading. The shared observation cache
keeps each block's projected sample by viewpoint and country mode, bound to the
recording/content key and charged for its retained allocations under the live
block budget. Re-asks clip retained samples to the moving prefix and share each
snapshot across the days it represents.
Prepared timelines retain complete days' numeric scores, availability and
auxiliary inputs per accepted recording block in the shared observation cache.
Viewpoint and side selection reuse those inputs; current group history still
classifies them on each reply. Partial days and days ending in gaps read their
reached hours directly. Amount reads find the hour's batch once, without
reconstructing each field's original run bounds.
Moment windows read their accepted prefix too. Its observation/side projection
is marked approximate while later history can revise side classification;
requested hours outside that prefix stay pending, with no invented scores.
