# dashboard-land-battles

The land battle products and the division log: land battle manifests, windows,
rankings, courses and terrain, and each division's combat history and totals.
Their behaviour is documented beside the code in
[products/land_battles.rs](src/products/land_battles.rs) and
[products/division_log.rs](src/products/division_log.rs), and in the widget
guides ([land battles](../../ui/src/widgets/land-battles/README.md),
[division log](../../ui/src/widgets/division-log/README.md),
[shared battles](../../ui/src/widgets/battles/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md). The
modules keep their dashboard paths, `products::<name>`, and name what they use
at the crates that own it (dashboard-base, dashboard-text and dashboard-core). The [dashboard](../dashboard/README.md)
re-exports them at `dashboard::products::<name>`, registers them, and prepares
and queries them in its pipeline; [dashboard-naval](../dashboard-naval/README.md)
reads the land battles' country filter.
`CENSUS_TABLES` declares every land-census input for the shared walk and direct reads.

The division log's block preparation, for the dashboard's family prepared on
its own, is in [products/division_log/block.rs](src/products/division_log/block.rs):
the block extract a pre-pass worker runs from the block's first hour, its fold
onto the carried scan, and the branch whose block windows, manifest and totals
are written at its next commit.

`cargo test -p dashboard-land-battles` builds and runs only these products'
tests.

The division log answers its manifest and committed windows before preparation
finishes. A requested window can also be proved from its block and predecessor.
Only creation candidates in the target window need the parallel key-only read
of earlier seen handles; candidates already seen are removed before rendering.
Losses and transfers need only the local census and adjacent drains. This
preserves the distinction between a creation and a reappearance without reading
older blocks for windows with no creation candidates.
The local scan needs an earlier complete census and dated presentation evidence;
otherwise the window stays pending. Template choices combine the prefix's
catalogue with proved newest rows; cumulative totals remain visibly preparing.
The compressed proof and its per-block earlier keys use the shared observation
cache and its live-block retention bound. A source revision owns them without
append inheritance, and recording/content binding changes invalidate them.
Moving preparation coverage reuses retained observations.

A proven newest land battle window publishes its page and window position before
its branch-wide variant naming diagnostics. Its look-back reads blocks ahead
and observes them in order. The dashboard runs the proof and later diagnostics
as work beside the forward families, each in its own group commit. Until the
diagnostics commit, the reply carries `warningsComplete: false`; a naming
refusal withholds only the diagnostics, leaving the exact proven rows available.

Land battle manifests and log windows fall back to the committed prefix while
the newest proof is unavailable. Before the first commit their ordinary replies
have no accepted observations or rows. A committed page's geometry is marked
approximate until complete, since later terminal reads can repair earlier rows.
The log follows the newest covered window and refreshes as its gate moves.

Newest-window look-backs and division-log manifests borrow immutable recorded updates from replay's shared section cache. Their reader frontiers contribute to its live block span; reconstruction and window proofs remain local to each scan.

Land battle duration changes reuse the preceding run's distinct participant
counts when an exact recorded delta leaves both rosters unchanged. Roster
edits, snapshots, gaps and newly opened combats count the recorded handles
again, including duplicate handles. This cache is the run's existing counts;
it adds no saved state.
