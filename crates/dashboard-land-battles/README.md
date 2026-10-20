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

The interactive combat-log-history product supplies the retained ledger to the existing casualties, equipment-losses and land-battles views. It keeps native history identities separate from replay episodes and resolves equipment and template labels through recorded naming operands and content. Empty months remain entries.
The request uses the shared dated country filter, including every manual pick
and smart-group or faction membership. Retained groups, months, manpower,
templates, battles and active/terminal observers have independent named source
availability. Expandable detail tables retain side countries, leaders, air
factors and pool gates, all observer counters, leader hours, attribution,
activity templates, division damage, equipment and aggregate losses.
Equipment labels reuse the shared supplying-read lookup and frame cache, with
the naming read's country and lexical operands on carried hours. Like battle
rankings, observer filters retain unknown participants; an empty native country
list is shown as unknown, and an absent terminal side stays absent.

## Army losses and reached-hour replies

`army-losses` ([products/army_losses.rs](src/products/army_losses.rs)) answers one
country's retained Combat Log at a reached hour for the
[Army losses by month](../../ui/src/widgets/army-losses-by-month/README.md) widget:
each army's retained months with the log's three signed manpower source channels
(Ground, Air, Navy; additive, not killed/wounded/captured) and each retained ledger
month's designs with their archetypes. Each army's log joins its orders group by
full reference; month rows carry the reference's id. Logs of army groups and of
armies that no longer exist are refused and counted, as the game's own page leaves
them out. Absent months stay absent: the reply never fills a pruned month.
Equipment rows join the months the game keeps at the hour, so a pruned month's
rows that linger until its country's next read are left out and counted. A
country's ordinary read holds only each array's last two month entries, and a
full read (the recording's start, a load, a gap, a refresh, or a country whose
last complete read is older) every retained one. An older month's amounts are
known at the hour when a read at or before it covered the month and every hour
from that read to the hour was read: they are what that read found, its rows or
none (an empty month). One backward walk over the branch's blocks settles every
older month of a reply together, from each block's index of the latest read
that covered each country's months and the hours the block did not read
([army_losses/ledgers.rs](src/products/army_losses/ledgers.rs)). A block that
ends by the hour is indexed whole for every country, streamed from its changes
and kept in the observation cache; the walk keeps no block once processed and
stops when every month is settled or an unread hour leaves the rest unknown. A
month written after the latest read, one no read covered, or one whose later
reads or pool were not all recorded is unknown, never zero.

[products/reached.rs](src/products/reached.rs) is the shared reached-hour cache:
a reply computed from one hour's frames, kept in the observation cache of the
block holding that hour under the source's recording and content binding and
the load's presentation (the folders, cache folder and unverified-content
choice, and the catalogue's languages and texture settings), so a reply is
never answered under another presentation. The
army products in [dashboard-army](../dashboard-army/README.md) use it too, along
with the division log's template artwork and division name groups.
