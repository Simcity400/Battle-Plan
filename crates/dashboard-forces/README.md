# dashboard-forces

The force-size products: army size, supply health, air force size and navy
size by country, and the parts-race families air and navy share to prepare and
query their series. Their behaviour is documented beside the code in
[products/army_size.rs](src/products/army_size.rs),
[products/supply_health.rs](src/products/supply_health.rs),
[products/air_force.rs](src/products/air_force.rs),
[products/navy.rs](src/products/navy.rs) and
[products/parts_race_prepared.rs](src/products/parts_race_prepared.rs), and in
the widget guides ([army size](../../ui/src/widgets/army-size/README.md),
[supply health](../../ui/src/widgets/supply-health/README.md),
[air force size](../../ui/src/widgets/air-force-size/README.md),
[navy size](../../ui/src/widgets/navy-size/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md). The
modules keep their dashboard paths, `products::<name>`, and name what they use
at the crates that own it (dashboard-base, dashboard-text and dashboard-core). The [dashboard](../dashboard/README.md) re-exports them at
`dashboard::products::<name>`, registers them, and prepares and queries them in
its pipeline. The crate implements `ReplyWarnings` for its four replies.

Air and navy preparation pass exact recorded row deltas to their native
censuses. A changed wing revisits its owner, effective owner and plane count;
a changed ship revisits its task force, strength and class. Other changed
fields do not recount an existing unit. Equipment and model deltas include
deletions; snapshots, gaps and changed joins or naming evidence rebuild the
census. Air equipment spellings hold until that rebuild. These transient
caches do not change the saved series or continuations.

`cargo test -p dashboard-forces` builds and runs only these products' tests.
The `test-support` feature compiles the army's synthetic frame and the force
families' synthetic recording and prepared store for the dashboard's pipeline
and query tests.

Supply preparation keeps each division's current supply bucket and each owner's
counts, including the number of divisions with missing operands. Recorded supply
and naval-transfer deltas replace only those divisions' contributions. Unrelated
status, land and ownership-table fields retain their projections. Snapshots,
gaps, availability or ownership changes rebuild the counts. Numeric publication
reads these counts directly; queries retain the shared sampler. These joins are
runtime state and rebuild after restore.

Supply health's reached-hour summary and history share a content-bound endpoint
observation, including availability and warnings. Its request binding includes
the content folders, cache folder and unverified-content choice, so tightened
verification runs the ordinary refusal checks before accepting an endpoint.
While preparation remains before that hour, re-asks reuse it across prefix commits and presentation widths;
once the prefix reaches it, ordinary prepared values supply the endpoint.
An uncovered single-hour view with recorded land inputs observes that reached
frame alone. A missing endpoint retains the ordinary fold's availability and
refusal evidence.

An air or navy block extract steps every hour's census, but makes output rows
and projection bases only after its scan settles and its fold will keep them.
The first hour and air's hours before its variant names settle keep only the
scan step and boundary state; their fold observes those hours itself.

Force censuses borrow recorded updates through `FrameSource`, accepting owned wire frames and immutable shared recorded frames with the same delta and baseline rules. Query scans share section payloads with preparation without copying on a cache hit.

Army, air and navy rank publication coalesces adjacent unchanged positions
before writing accumulator runs. Their channels share numeric payload
construction within a flush, while retaining each section's ordinary bytes
and continuation. Each family also retains its current published catalogue
text and compressed ranges, avoiding a reread at the next flush; restoration
validates that chain again. Air and navy country joins follow recorded
balance-only field deltas without comparing every country's identity again.

Catalogue reuse checks every live range's immutable identity and the next
empty/missing slot in the current writer. A second publisher, store
replacement or abandoned writes invalidate the cache; an unchanged append
can retain it. The shared publication rules are in
[dashboard-base](../dashboard-base/README.md#prepared-series).

Factories and army share a sorted rank-boundary plan across their channels;
army borrows the native run operands while clipping only their rank intervals.
