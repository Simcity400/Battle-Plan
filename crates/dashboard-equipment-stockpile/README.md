# dashboard-equipment-stockpile

The equipment stockpile product: what one country holds of each variant over a
branch, what its production lines make of it, and the dated best-variant
artwork. Behaviour is documented beside the code in
[products/equipment_stockpile.rs](src/products/equipment_stockpile.rs) and in
the widget guides ([equipment
stockpile](../../ui/src/widgets/equipment-stockpile/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md),
laid out like
[dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the module
keeps its dashboard path and names what it uses at the crates that own it
(dashboard-base, dashboard-text and dashboard-core). The
[dashboard](../dashboard/README.md) re-exports it at its old
`dashboard::products::` path, registers it, and prepares and queries it in its
pipeline.

Prepared holders catch up through an authenticated span in one step, retaining
the same open-run endpoints and saved state as advancing every hour. Identity
changes and breaks are observed at their own hours between those spans. Numeric
reads use recorded row deltas when present; a changed table without them,
or a change to table availability, holders or dictionary, rebuilds the whole
projection. Continuation JSON uses the hourly branch writer's shared buffered
compression helper.

Within a continuing holder's coverage, numeric projection and pruning visit
only the keys whose value interval started or stopped. Equal reads retain their
dated read events and diagnostic counts without projecting unchanged numeric
keys. Group projection follows the pending interval changes shared with numeric
publication. Equal members, coverage and classifications carry the previous
projection without visiting its groups. Learned logistics spellings invalidate
that carry, as do variant reclassification and content rebinding. A changed
member selects its type and domain; unresolved operands, changed completeness, classifications or coverage
select every group they can affect. Runtime coverage proofs clear on content
rebinding and rebuild after restore. Numeric change collections also rebuild
after restore; neither is part of the saved state.

`cargo test -p dashboard-equipment-stockpile` builds and runs only this crate's
tests.

A table sampled before complete preparation retains exact supplying-read
quantities, with its type attribution marked approximate at the requested hour.
The complete branch can supply a later nonempty classification token. The table
shows this distinction without changing the supplying-read sampler.

The at-hour sampler also answers before the first commit, with the shared empty
accepted extent. Its raw reply is retained in the existing observation cache,
bound to content and keyed by holder tag, hour and membership spans. Re-asks
update preparation metadata without repeating the supplying-read look-back.
The retained-byte bound follows the live blocks in that read window.

Naming carries the shared catalogue's country-key proof through unchanged hours
and retains recorded field deltas at a complete read. Only changed naming
operands rebuild raw variant presentation; a pending or restored read rechecks
its full inputs before carrying them.

Kept frames without recorded row deltas compare each changed numeric source
table once and mark only the owners of differing rows. Those owner marks
accumulate until their stock or logistics supplying read. Dictionary changes
compare the logistics spellings in use and mark their owners for the next
logistics read; unrelated dictionary additions carry every holder. Equal reads
still emit their own dated read events and diagnostic counts. Phase labels
separate source comparisons, spelling invalidation, changed holders, whole
holders and repeated reads.
