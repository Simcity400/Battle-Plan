# dashboard-intelligence

The intelligence products: agency comparison, the espionage log, decryption,
the operatives roster and operative placements, with the intelligence and
operative content they read, the operation descriptions, and the intelligence
observer that prepares all five from one pass over each frame. Their behaviour
is documented beside the code in [products/](src/products/) and in the widget
guides ([agency comparison](../../ui/src/widgets/agency-comparison/README.md),
[espionage log](../../ui/src/widgets/espionage-log/README.md),
[decryption](../../ui/src/widgets/decryption/README.md),
[operatives roster](../../ui/src/widgets/operatives-roster/README.md),
[who spies on whom](../../ui/src/widgets/who-spies-on-whom/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md). The
modules keep their dashboard paths, `products::<name>`, and name what they use
at the crates that own it (dashboard-base, dashboard-text and dashboard-core). The [dashboard](../dashboard/README.md)
re-exports them at `dashboard::products::<name>`, registers them, checks their
content, and prepares and queries them in its pipeline. The crate implements
`ReplyWarnings` for its replies.

`cargo test -p dashboard-intelligence` builds and runs only these products'
tests. The `test-support` feature compiles their test content for the
dashboard's content-check and pipeline tests.

First views read committed history immediately, including an empty prefix before
any accepted hour. Day requests beyond it read the supplying block and its
predecessor through the shared frame cache: agency screen values, cipher amounts
and countdowns, and complete operative placements are exact at the requested
hours. Cipher attempt starts and breaks, and roster first presence and departure,
remain preparing. Manual agency selections needing identity history use the
joint prepared prefix. The espionage log proves its newest day from the day and
supplying blocks when their episode boundaries are witnessed locally. Inherited
episodes and unmatched late completions for earlier handles remain pending.
Day replies, per-block earlier keys and compressed newest proofs use the shared
source-revision observation cache. Recording/content binding changes invalidate
them; moving preparation coverage clips their use. Roster day cache keys also
include accepted identity coverage, so a pending selection is reconsidered as
that history advances. Each product shares the cache's live-block retention
bound across its request variants, without a separate
screen budget. Coverage re-asks reuse observations while retained.

The cold operative roster selects holders named for the requested tags by the
day's identities and accepted identity history. A later alias can add a holder
to an earlier day's selection, so only identity coverage complete for the whole
branch makes that selection final. Until then the reply names `countries` as
preparing and supplying reads leave pending hours intact. Prepared day values
carry the same provisional-selection mark while identities are incomplete.
The reply publishes the accepted identities extent as `selectionCoverage` (null
before acceptance). Mounted clients replace the roster when that extent moves,
even if numeric coverage and the pending/preparing marks stay unchanged. Its
reply gate follows incomplete identities as well as intelligence preparation.
The roster projects known selected holders and their aliases through the shared
frame cache. Its rows and string dictionaries use the same holder selection
before reply construction and budget checks. The reply carries the witnessed
identity spans covering its roster observations. Join and departure dates remain
preparing until the joint intelligence and identity prefix reaches them.

Country identity comparisons follow recorded country field masks: balance-only
deltas and equal acquisitions keep the previous key projection. Snapshots,
identity edits and membership changes use the complete key comparison.

Agency comparison contains a country-intel section. Its `intel-details` product reads the demanded hour and, when needed, fuel's latest supplying hour through the shared selected-table frame cache. Country and viewing-country choices include every recorded country; reserved slot-zero sources and raw-zero propaganda holders are labelled by their actual slots. It shows source pools/current/prior values and references, network auxiliary/state details and logical propaganda history. Recorded content defines supply caps and exact daily accumulator decay.

Player estimates are computed for the selected viewer/target from recorded game state and content, independently of the recording player's panels. The range is deterministic from the recorded seed/month, metric/category, true quantity and normalized intel. Getter lanes, caps, inclusive thresholds, Q15 conversion, integer jitter and rounding follow native arithmetic. Self/alias, direct-war veto, common faction and either subject direction supply full-information context. An unrecorded fog switch refuses a result only when its possible values disagree. Fuel ranges use the latest CountryFuel supplying hour, with all other operands evaluated from that same hour. Untimed carried keyframes recover the actual acquisition through the shared daily supplying-read query, bounded to one cadence and validated across intervening blocks. The table and tooltip label the supplying date and hour; missing reached operands there are refused by name. Daily flow sums and demand rates cannot reconstruct an intervening tank level. The sliced truck cache likewise requires its owner's current-hour read. Missing reached truth, category, content or freshness operands are named and counted; true totals are not substituted as estimates. No UI hooks, holders or caption table exist. Operatives hover behaviour is unchanged.

Equal country and link acquisitions retain their shared identity join without
comparing every link. Each product still receives its own supplying-read dates
and accumulates membership and operation invalidation until its next read.

Intel network coverage has a separate sparse keyed [summary module](src/products/intel_coverage/README.md); the five-product intelligence observer remains unchanged.
Its windows carry dated membership and network-count ranking runs, not a
final-hour roster. Sparse pair histories cover countries selected anywhere in
the window, with unknown inventory excluded from ranking and absence proof.
