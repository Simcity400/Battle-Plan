# Intel network coverage

`IntelCoverage`, `IntelCoverageObserver` and `FAMILY` add a separate keyed family
inside the existing owner without modifying its intelligence observer. Store and
product: `intel-network-coverage`. Only `IntelNetworkSummaries`, `Countries` and
`CountryLinks` are selected. The summaries' daily/forced read dates the values.

## Wire contract

Both matrix and pair views return `{window, warnings}`. Countries are `TAG@slot`.
Pair series are `OWNER|TARGET|measure`, with measure `coveredControlled`,
`coveredCore`, `coveredWorth`, `controlledStrength`, `coreStrength`, `maxStrength`.
The first three are signed integer counts; the last three are signed fixed5
values (scale 100,000). All remain lossless decimal text, without clamping,
recomputation, percentages or inferred creation dates. A recorded empty network
emits all six zero values. Both directions are independent.

`member|COUNTRY` holds `1` while that identity is admitted at a dated observation.
Matrix replies also contain `rank|COUNTRY`: the count of networks owned plus
targeted, including empty networks, only while complete dated inventory and that
country's membership are known. Missing rank runs outside known coverage are
unknown, not zero. Rank is a count, not an ordinal. Playback selects its own top
countries by descending count, then country key for ties, and excludes identities
without a member run at that hour. Pair replies retain only the requested
direction's values and dated endpoint membership, with no rank series.

Parameters: `branchId`, optional `firstHour`/`lastHour`, `view` (`matrix`, default,
or `pair`). Matrix accepts optional `tags` (an explicit empty list selects none)
and `shown` (default 12; no arbitrary maximum). Pair requires `owner` and `target`.
Selectors are bare tags or exact `TAG@slot`; tag reuse stays separate. Explicit
tags narrow the ranking roster; a network still counts when its other endpoint
is outside that selection.

Ranking sweeps sparse pair-presence runs rather than seeking every hour. Pair
history includes existing series among the union of countries in the top `shown`
at any accepted hour in the window, plus every explicitly selected identity.
Earlier top countries and their history remain available when later leaders or
membership differ. A future or unaccepted hour cannot add countries to this
union. Catalogue storage is O(countries + recorded networks + changes), not a
dense world-pair/time cube. Narrow the window/selection if the complete reply
exceeds the shared reply budget; nothing is silently truncated.

## Evidence

An absence certificate requires a complete summary-table read: its native reader
walks every admitted non-reserved country's network inventory and emits a summary
for every actual network, including empty ones. The country/link tables must
also authenticate every owner/target and live displayed identity. Membership uses
the shared keyed accumulator; compact inventory transitions stay in observer
state. `window.known` is narrowed to complete dated inventory. A missing matrix
pair means no network only there and while both endpoints have member runs;
absence of an identity at another date does not erase earlier coverage. Pair
requests additionally require both selected endpoints to be admitted at that
hour before certifying absence. Present dated runs remain independently
exact even outside these narrower absence-proof spans, e.g. when another pair has
an unresolved endpoint. Missing runs outside known spans mean unknown, not zero.

The supplying join is frozen; identity changes before a fresh summary read revoke
it. Untimed baselines and post-gap carries stay unknown until the next witnessed
read. The six retained caches are never reconciled with state rows or auxiliaries.
No state/subnetwork detail, percentage, cap or label is invented. Country labels,
flags and colours use the existing dated content/catalogue service at the read
hour; numeric replies need no content lookup. Named refusal counts describe
network observations in the published catalogue, not network lifetimes.

The lead registers the Product/types, keyed FAMILY/Scan fold, query, gate and
supplier cache; dispatcher prefix marking is independent of capture evidence.
Tests cover exact signed caches, empty/absent/unknown, directionality, stable dated
ranking and changing leaders, independent identity refusals, tag reuse, gaps,
equal forced reads, serialized continuation, accepted-prefix clipping and 5,000
sparse countries. The round-1 tests sweep the full hour domain without hourly
work. Their targeted filter is `intel_coverage_r1_`.
