# dashboard-naval

The naval products: naval battle manifests, windows, rankings and details,
naval losses and ships sunk, all built on one retained sinking scan. Their
behaviour is documented beside the code in
[products/naval_battles.rs](src/products/naval_battles.rs),
[products/naval_losses.rs](src/products/naval_losses.rs),
[products/ships_sunk.rs](src/products/ships_sunk.rs) and
[products/sinkings.rs](src/products/sinkings.rs), and in the widget guides
([naval battles](../../ui/src/widgets/naval-battles/README.md),
[naval losses](../../ui/src/widgets/naval-losses/README.md),
[ships sunk](../../ui/src/widgets/ships-sunk/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md) and
[dashboard-land-battles](../dashboard-land-battles/README.md), whose country
filter the naval battles share. The modules keep their dashboard paths,
`products::<name>`, and name what they use at the crates that own it
(dashboard-base, dashboard-text, dashboard-core and dashboard-land-battles). The
[dashboard](../dashboard/README.md) re-exports them at
`dashboard::products::<name>`, registers them, and prepares and queries them in
its pipeline. The crate implements `ReplyWarnings` for its replies.

A scan whose content is bound publishes a block from the recording alone
(`naval_battles::Scan::publish_configured`) into any page sink: the
dashboard's naval family, prepared on its own, publishes each block at its
end into pages held for its next commit. Its
frames carry `naval_battles::PREPARED_TABLES`, and the trigger kit that sunk
ships' scripted place names evaluate at the hours a sinking can first be read
at; `Scan::observe_details_read` observes a frame without the kit, where a
ship first found stops preparation
(`ships_sunk::battle_titles::Prepared::observe_read`).

A branch's newest naval battle window is prepared before forward preparation
from its last block and a look-back proven to hold every fact its page carries
([products/naval_battles/newest.rs](src/products/naval_battles/newest.rs)),
with the earlier blocks read only by the changes of their combat keys, result
keys and sources; the dashboard publishes it first and answers the manifest and
that window from it until the naval family covers the branch.

`cargo test -p dashboard-naval` builds and runs only these products' tests.
