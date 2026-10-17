# dashboard-air-aces

The air aces product: every country's ace roster, dated by successful reads,
with each ace's wing, aircraft and death, and the ace portraits. Behaviour is
documented beside the code in [products/air_aces.rs](src/products/air_aces.rs)
and in the widget guides ([aces](../../ui/src/widgets/aces/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md),
laid out like
[dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the module
keeps its dashboard path and names what it uses at the crates that own it
(dashboard-base, dashboard-text and dashboard-core). The
[dashboard](../dashboard/README.md) re-exports it at its old
`dashboard::products::` path, registers it, and prepares and queries it in its
pipeline.

Ace preparation restores active roster baselines from the shared resume page
when it contains the ace table at the saved hour. Otherwise it seeks that hour's
campaign and ace tables from the recording before continuing.

`cargo test -p dashboard-air-aces` builds and runs only this crate's tests.

Prepared `air-aces` also accepts `view: roster` with an `hour`. A reached hour
uses the prepared manifest and window. Beyond coverage, the shared supplying-read
lookup finds the latest complete roster within its daily cadence, without
crossing a gap or missing table. The ordinary roster builder supplies current
names, owner, wing, aircraft and alive/dead state. `firstSeen` is null and
`preparing` names first-sighting and death dates until history reaches the hour.
Lazy portraits use that same supplying read. The root carries coverage and
pending hours so mounted rosters are replaced as preparation advances.

The cold roster also resolves the latest variant-name read within its own daily cadence before the roster read, preserving the aircraft names carried by hourly preparation.

Country identity comparisons follow recorded country field masks: balance-only
deltas and equal acquisitions keep the previous key projection. Snapshots,
identity edits and membership changes use the complete key comparison.

Country identity invalidation accumulates at every accepted hour, including
hours without a roster read. Identity edits, membership changes, snapshots,
gaps and unavailable countries require the next roster read to compare the
full country keys before carrying owners and portrait operands. Balance-only
deltas check their masks without walking retained country keys.
