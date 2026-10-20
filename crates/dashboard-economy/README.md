# dashboard-economy

The economy widgets' prepared families and their queries, above
[dashboard-base](../dashboard-base/README.md), laid out like
[dashboard-politics-ledger](../dashboard-politics-ledger/README.md). The
[dashboard](../dashboard/README.md) re-exports each module at
`products::<name>`, registers it, prepares it on its own in the block
pre-pass's shared read and routes its queries through the prepared prefix.

| Module | Family | Queries | Widgets |
|---|---|---|---|
| [fuel_branches](src/products/fuel_branches.rs) | `fuel_branches` | `fuel-branches` | Fuel by branch |
| [market](src/products/market.rs) | `market` | `market-history`, `market-ledger` | International market, Market ledger, Lend-lease agreements |
| [lend_lease_balance](src/products/lend_lease_balance.rs) | `lend_lease_balance` | `lend-lease-balance` | Lend-lease balance |
| [production_catalogue](src/products/production_catalogue.rs) | `production_catalogue` | `production-catalogue` | Production licences, Industrial organisations, Equipment designs |
| [convoy_health](src/products/convoy_health.rs) | `convoys` | `convoy-health` | Convoy health |

## How the families keep their data

Each family names its values by a field (its native holder and what it
holds) and keeps them as coalesced runs in the shared prepared series
([keyed.rs](src/products/keyed.rs)). An hour is observed whole when a watched
table's rows changed, a table was read in its slot, or the hour follows a gap;
every other hour carries the held values in constant work. A value starts at
the first read showing it and ends at the first read that does not; a gap ends
every value, and an unreadable table leaves its values unknown (no run) with a
named, counted refusal, never zero or an empty list. Each table's
completeness that a widget needs to tell "not recorded" from "none" is its own
run. Holders are native canonical country slots with their tag runs, so a
query by tag reads only the hours a holder had that tag.

Market occurrences are kept once, in their kind's occurrence stream. Each party
holder keeps a compact dated index per kind (the occurrence's sequence across
kinds and its stream position), so keys grow with kinds, holders and volume,
never with trading pairs, and an idle flush republishes no pair history.
Streams and indexes are chains of keys of at most `CHUNK` (512) entries, each
key named by its chunk and first hour: the shared series decodes a key's whole
blob in each batch a read touches, so a chain bounds what any read decodes.
A ledger merges segments (a chain over the hours it applies) newest first by
sequence: every country's ledger one per kind's stream, a chosen-country
ledger one per chosen holder, kind and span the holder had a chosen tag; an
occurrence two chosen parties share has one sequence and is taken once. Its
cursor is each segment's chain, first hour and position, so a page decodes
the chunks holding its own index entries and rows plus a count and a head per
segment, never the selection's whole history, and later occurrences never
shift it. Every country's ledger names its exact total from the streams'
lengths; a chosen-country ledger has none. A group is asked as its members' dated
memberships (`members`): each member's holders over the hours they held its
tag inside its own hours. The ledger also returns when the market's occurrences were
read, so an empty ledger tells no trades from not recorded. Queries with chosen
countries read each row's party key first and skip rows no chosen country is
party to before reading the rest. Each domain
also publishes when its table was read complete (`Known`), so an unreadable
roster reads as not recorded rather than empty, and a selected policy the
recording cannot spell stays apart from no policy. The query routes are
registered with the observation reply cache, bound to their family's
publication. Equipment names come from the
shared variant catalogue at their own naming reads; a family publishes only the
variants it names.

Queries read only the requested view's keys through the gate's prepared
prefix; the hours after it are pending, never empty. Content (names, icons,
region and resource names) is resolved at query time through the recorded
content stack, so numeric facts survive a name that refuses.

## Check

`cargo test -p dashboard-economy` builds and runs these families' observer and
query tests on synthetic frames, including mod-independent identities, gaps,
refusals and 5,000-country sparse inputs.
