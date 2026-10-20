# dashboard-politics

Country political state: election schedules, governments in exile, balances of power and civil-war rivals, for the [Next elections](../../ui/src/widgets/next-elections/README.md), [Governments in exile](../../ui/src/widgets/governments-in-exile/README.md), [Balance of power](../../ui/src/widgets/balance-of-power/README.md) and [Civil wars](../../ui/src/widgets/civil-wars/README.md) widgets. Behaviour is documented beside the code in [products/politics_state.rs](src/products/politics_state.rs), [history.rs](src/products/politics_state/history.rs) and [civil_wars.rs](src/products/politics_state/civil_wars.rs).

This is a product crate above [dashboard-base](../dashboard-base/README.md), laid
out like [dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the
module keeps its dashboard path, `products::politics_state`, and the code names what it uses
at the crates that own it ([dashboard-base](../dashboard-base/README.md),
[dashboard-text](../dashboard-text/README.md) and [dashboard-core](../dashboard-core/README.md)).
The [dashboard](../dashboard/README.md) re-exports it, registers it, and prepares and queries it in its pipeline.

`cargo test -p dashboard-politics` builds and runs only these tests.

## Products

| Key | Kind | Answers |
|---|---|---|
| `politics-state-history` | prepared (family `politics_state`) | Every holder that held a country's tag (whole), every holder's balances, every election schedule over the branch, or every government in exile over one requested window, as coalesced runs through the prepared prefix, with the hours the part was read. |
| `civil-wars` | prepared (joint with `event_feed`) | App-owned civil-war episodes from each holder's stored rival list, each with the hours the game's own civil-war predicate held for a side (ongoing while it holds), and an outcome only where a civil-war annexation of every other side is recorded; with the hours the lists were read. |
| `politics-state` | interactive | One hour's detail the history does not keep: exiles' deployed air crews, and every stored balance record with its members' recorded daily and weekly contributions and the weekly drift's static-modifier sources. |

## Rules

- Each part (elections, exiles, balances, civil wars) is observed at an hour that changes what it reads, by field (the fields of the rows it reads, a war relation's end arriving, and the identities naming its countries), or that follows a gap; at every other hour it carries, so a change elsewhere (a war's casualties, a country's political power) costs it nothing. Each observation is the part's whole state at that read, so a value starts at the first read showing it and ends at the first read that does not. A gap or an unreadable table leaves values unknown (no run), never zero.
- Elections use the recorded game predicate and the shared native calendar step ([country_people::next_election_date](../dashboard-base/src/products/country_people.rs)). A known "not held" and an unrecorded eligibility stay apart; an initial read establishes a schedule, never an election.
- Exiles use the exile cards' own projection ([country_people::exiles](../dashboard-base/src/products/country_people.rs)); each field and its supplying read are independent runs. The first read in exile is a bound, not the exile's start.
- Balances keep the game's stored record order: a country's shown balance is the first record listing it, which needs the whole membership resolved. Records have no identity beyond their position, so history follows each holder's membership by balance identity, with the record's position at each read: holders sharing it at one hour share that record, and equal values never join records. Names, sides and ranges resolve through the recorded content stack; a range whose bounds the stack does not establish (undeclared, or an ambiguous duplicate id) keeps null bounds and every other fact, and a name that does not resolve leaves the numbers exact.
- Drift sources derive from the record's recorded static-modifier roster through [replay_derive::power_balance_derive](../replay-derive/src/power_balance_derive.rs) and are kept apart from the members' recorded contributions, without forcing them to reconcile. A source the stack does not declare refuses the list by name.
- Civil-war lists are kept as the game stores them: one country's list never answers another's. The game's civil-war predicate is asked of every listed holder at an observation through one index of its inputs ([dashboard_text::identity::CivilWars](../dashboard-text/src/identity.rs)), with the answers and refusals of the game's own scan. Annexation by a country that was not a side, a later split, a gap or a capitulation decides nothing; peace settlements are not yet joined.
- Each part keeps the hours its tables were read; a value ending where its part was not read ended unknown, never then.
- Holders keep their tag runs. A country query returns every holder that held the tag, each whole, so a view follows the holder holding it at its date through renames and never joins a holder that reused the tag.
- The exile history answers one requested window: every exile's per-read runs over a whole branch outgrow a reply at supported scale.
