# collector-ledger-tables

Event store tables: the hourly packs that drain the event feed history and the air loss, building bombing and nuclear strike event ledgers in collector-observers into owned rows and convert them. The host owns dispatch and table assembly.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-ledger-tables` and `cargo clippy -p collector-ledger-tables --all-targets -- -D warnings` for its synthetic checks.

The market pack drains fixed holder marks and rare final snapshots on the existing worker, derives equipment total changes, strips intermediate snapshot/pointer records, assigns output IDs and preserves public market/lend-lease occurrences and publishes changed-owner updates for country holdings and exact changed contract/agreement keys. Agreement equipment pools share immutable storage with the held and encoder baselines; fuel changes publish scalars and pool handles only. Conversion consumes the owned changes once, without cloning the pack. Independent state refusals leave the other state tables and complete occurrences available. Unchanged owners use certified baselines; events retain terminal transfers after native removal.

Contract updates use exact holder keys, including removals, and share carried
collections through conversion and encoder baselines. Country-reset occurrences
apply after prior terminal differences. Missing delivery amounts produce named
holder refusal rows in a complete MarketEvents row stream. Conversion never
marks that stream truncated for a holder-level amount refusal, so periodic and
forced keyframes preserve fuel, payment, removal and refusal rows together. A state refusal
alone still does not refuse independently complete events.

Cold attach and post-gap recovery publish all three market state baselines from
the fresh census epoch, independently of a lost event interval and the caller's
full flag. Only MarketEvents keeps a true event-interval refusal. Empty baselines
are complete too. The admission regression validates every state field and nested
value through frame encoding, spool-record encoding, decoding and keyframe seek,
then verifies complete event rows once observation resumes.

After a spool-only loss the admission regression requires the next intact hour
to retain complete events and carried state, with no second census. Separate
regressions preserve failed daily/off-slot contract census attempts without
re-reading their manager or header in the same epoch.

Populated native admission fixtures cover country zero at cold startup and its
ordered replacement, contract active removal before deferred destruction,
shared variable notifications and holder-address reuse. Native retired identity
words and cleared registration verify that deferred cleanup emits no duplicate
terminal occurrence. The production capture/projection and spool validation paths
check exact current rows and terminal increments. Missing births and failed
selected identities stay named;
quiet acquisitions use a panic-on-read memory fixture.
