# dashboard-unit-leaders

The unit leaders product: every military leader's career, skills and traits from
hourly reads, and their portraits. Behaviour is documented beside the code in
[products/unit_leaders.rs](src/products/unit_leaders.rs) and in the widget
guides ([generals](../../ui/src/widgets/generals/README.md), [general
career](../../ui/src/widgets/general-career/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md),
laid out like
[dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the module
keeps its dashboard path and names what it uses at the crates that own it
(dashboard-base, dashboard-text and dashboard-core). The
[dashboard](../dashboard/README.md) re-exports it at its old
`dashboard::products::` path, registers it, and prepares and queries it in its
pipeline.

The family is prepared on its own: [products/unit_leaders/block.rs](src/products/unit_leaders/block.rs)
holds its pre-pass extract (a block read by the fields leaders read, with the
tables its run binds from the content, kept as whole, reduced and quiet hours
with the block reader's change facts) and its
fold on the carried prepared state, described in the
[dashboard's prepared leaders](../dashboard/README.md#prepared-leaders).
Content-bound reads select the trigger inputs listed in the narrow block
policy. Script variables keep their entries and seed; country tag resolution
watches countries' slots and tags, links, owned-state counts and original-tag
buckets. Kept counter rows retain only their key and owned-state count.
Unlisted inputs remain dropped and require whole-frame observation when read.

Measured CPU notes count each fold's fallback site as `leaders.fallback.whole`,
`.kept`, `.built` or `.quiet`, and its rejected proof as `leaders.reject.*`.
Kept visibility counts only the first rejection in an hour. Quiet proofs name
command continuity, frontier, reads, tables, content, core completeness or
visibility inputs separately. `leaders.rejected-table` CPU labels name the
tables whose declared reads defeated that proof; `undeclared-input` and
`input-hour` notes distinguish an undeclared read from inputs last confirmed at
another hour. `leaders.undeclared-input` labels name the exact undeclared query;
it has no declared table or fields. `leaders.rejected-input` labels name each
rejected declared query, its table and its field names. These diagnostics do no formatting or input-key walks
outside CPU measurement and do not change observations or saved data.

`cargo test -p dashboard-unit-leaders` builds and runs only this crate's tests.
The dashboard's slow shared-extract regression covers variable and explicit-tag
visibility, one shared decode per input table, no family-thread decodes and
byte-identical publication against hourly preparation.
The `test-support` feature compiles the outcome frames
(`prepared::test_fixtures`) the dashboard's pipeline tests build on and the
whole-recording loader, the parity oracle they compare against.

A prepared leaderboard request may add an `hour`. Beyond the accepted prefix,
`frame_at` supplies that hour to the ordinary leader observer, including its
visibility and content evaluation. Rank, skill, attributes, traits and portrait
selection are current facts. Battle arrays, trait-gain dates and commands are
withheld with `preparing: ["battles", "history"]`; the UI keeps their counts null,
disables history actions and waits for battle-count sorts. Reached requests use
the prepared history, and root coverage/pending fields keep mounted requests
following preparation. Other history requests answer their prepared prefix.
Country-filtered requests wait for complete identity history to resolve selected
native holders and their aliases, including transfers and reused tags.
