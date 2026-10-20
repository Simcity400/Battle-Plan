# Occupation reserve manpower

`OccupationReserve`, `OccupationReserveObserver` and `FAMILY` are an independent
keyed family in this existing owner; the occupied-state observer is unchanged.
Store: `occupation-reserve`. Product: `occupation-reserve-manpower`. The dating
supplier is `CountryOccupationReserveManpower` (daily slot and witnessed forced
reads); the other selected inputs are `Countries` and `CountryLinks`.

## Wire contract

Both views return `{window, holders:[{tag,slot}], sources:{id:country}, warnings}`.
`country` is `TAG@slot`. Series names are `country|ordinal|amount` and
`country|ordinal|source`. Amounts and source ids are decimal text in exact keyed
runs. `sources` is necessary to decode the source ids: the shape addendum names
this dictionary but omits it from its example race object. Source dictionaries
are filtered to ids used by the returned window. Repeated sources retain every
native ordinal, including signed zero and negative entries.

Parameters: `branchId`, `view` (`race`, default, or `holder`), optional
`firstHour`/`lastHour`; race accepts optional `tags` and `top`; holder requires
`tag`. Country selectors accept either a bare tag or an exact `TAG@slot`.
Bare tags keep reused identities separate. An explicit empty race selection
selects nothing. `top` ranks at the last accepted requested hour using positive
amounts only, ties by country key, then reads only those holders' histories.
Without `top` all selected holders are accessible; an oversized complete reply
is refused, never truncated. Negative entries never reduce race rank/bar length;
the UI must retain them separately in the tooltip/drill. Zero occurrences remain.

`window.reads` dates the actual supplying reads, including equal forced reads.
Untimed baselines and carries after gaps cannot supply values until a read is
witnessed. Identity changes between supplier reads revoke the join rather than
rebinding an old source to a new country. Country labels, flags and colours are
not embedded: resolve them through the existing dated country-label/catalogue
service at the supplying hour. No content lookup or occupation-law evaluation
can suppress these numeric observations.

## Admission and refusals

These allowed tables contain **no per-holder occupation-admission witness**.
`window.known` certifies a complete dated reserve table, not eligibility of a
rowless holder. Missing holder occurrences therefore remain unknown even in a
known span; never synthesize a zero/empty holder. A warning states this explicitly.
Recorded zero rows are known zero. An unresolved source keeps its amount and
ordinal but has no source run; unresolved holders cannot be named and are counted.
Unavailable identity tables revoke that observation; unrepresentable tags are counted locally without withholding other countries' amounts. Counts in named
warnings refer to occurrence observations in the published catalogue, not unique
transfers. No transfer dates, negative-entry interpretation, garrison totals or
law explanation are derived.

The lead registers the Product, exported types, FAMILY/Scan fold and query in the
shared keyed registry, dispatcher, gate and supplier cache. Prefix marking remains
with that dispatcher. No changes to the older occupation observer are needed.

Tests use synthetic prepared stores, native/mod identity spellings, repeated
sources, signed extremes, unknown/empty distinction, fresh reads, gaps,
continuation/prefix clipping and 5,000 sparse holders. Run
`cargo test -p dashboard-occupation occupation_reserve` with the lane target.
