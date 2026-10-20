# dashboard-doctrine-mastery

Prepared hourly doctrine mastery for one requested country, above
[dashboard-keyed](../dashboard-keyed/README.md) and
[dashboard-base](../dashboard-base/README.md). The lead owns dashboard registration
and the widget. History requests read prepared pages only; selected-hour
scoped descriptions use the shared frame cache. Icons load separately through
the shared `sprite-artwork` route.

## Public API

`products::doctrine_mastery` exports `DoctrineMastery` (`Product`, key
`doctrine-mastery`), `DoctrineMasteryObserver` (`dashboard_keyed::Observer`, store
`doctrine-mastery`), `FAMILY`, and `query(&dashboard_keyed::query::Query)`.
`DoctrineMastery::types()` includes every nested reply type and keyed window type.
The ordinary loader keeps the shared prepared-only refusal.

The observer reads DoctrineCountries, DoctrineFolders, DoctrineTracks, Countries,
CountryLinks and LexicalTokens. DoctrineTracks supplies the hourly read date.
A doctrine-array slot joins the country's canonical slot, and its country reference
must match directly or through CountryLinks, as in the existing Doctrines product.
An unassigned/missing slot is not a country's recorded empty doctrine inventory.
Missing required tables make the observation unknown. Missing token spellings do
not withhold numbers; token ids with unavailable spellings are absent from `tokens`.
Interning includes the observed spelling, so a later spelling never relabels an
older unresolved id. Intern state survives publication/resume.

## Wire shape and names

Parameters: `{branchId, tag, firstHour?, lastHour?, hour?}`. Omit `hour` for
prepared history. A selected transient request may add `hour` for that exact
reached country's descriptions, without changing the window or name grammar. Reply:

```text
{window, folders:[{id,name,icon,grand:[{id,tokenId,name,icon}],
  tracks:[{id,name,subdoctrines:[{id,tokenId,name,icon,
    rewards:[{id,name,description,threshold}]}]}]}], tokens, warnings}
```

`window` is the shared KeyedWindow, restricted to the requested tag (each reused
canonical holder remains a separate `TAG@slot`), requested hours and accepted
prefix. No future/pending numeric value is served. Dashboard dispatch adds the
shared coverage/pending fields. Before first publication the same reply has an
empty window, folders and token dictionary.

Names, with literal `|` separators:

- `TAG@slot|folder|track|mastery`, `bank`, `daily`, `effective`, `rewards`, `sub`
  (one final measure per name).
- `TAG@slot|folder|grand`.
- `TAG@slot|present` witnesses a valid assigned doctrine slot even when its
  inventories are empty; `TAG@slot|folder|completed|ordinal` retains ordered
  completed subdoctrines, including repeats. These inventory names are unchanged.

Folder and track are **recorded ordered indices**, also the corresponding tree
`id` strings, not country references. Those are the schema's track identities;
there is no recorded authored track token. Folder indices preserve native content
order. The token dictionary spells interned ids in sub/grand/completed series and
known folder inventory tokens. Zero in sub/grand means no selection, not an
unknown spelling. All token-bearing nonzero values are interned ids; unresolved
ones are not given fabricated spellings. Grand and subdoctrine `tokenId` values
retain their recorded interned ids (null for unwitnessed authored alternatives).
Their `id` is the known token spelling;
without a spelling it is the numeric interned id, distinguished by `tokenId` and
its missing dictionary entry, never a display label or an authored-token lookup.
Reward choice ids are authored content tokens. Names are localised strings or
null on a named refusal, never internal-code display fallbacks. Unknown reward
inventories are null; only a known definition with no rewards gives an empty list.
Reward descriptions are null without a catalogue, independently of requirements.

Mastery, bank, source `daily`, `effective` and authored thresholds are raw signed
fixed5 decimal text: **100,000 raw units per unit**. `rewards` is an unscaled signed
count. No source gain is substituted for the effective gain. Unlock state comes
from the recorded rewards count and authored reward order, not a threshold test.
No unlock dates, completion dates, ETA, ceiling or folder-wide bank is inferred.
Within known observations, absent series mean absent entries; outside known spans
they are unrecorded. Gate folder/track interpretation by their containing
`grand`/`sub` presence and country inventories by `present`.

## Content and refusals

All definitions, names, reward requirements and descriptions use the recorded
content stack's shared doctrine Database, localisation, define reader and shared
benefit formatter. Reward labels use subdoctrine/reward composite keys, and
optional generated descriptions use the same generated-key rule as the existing
Doctrines panel. Threshold zero uses NDoctrines.DEFAULT_REWARD_MASTERY, never a
hard-coded gameplay default. Any number of unequal thresholds remains in authored
order. Nonpositive requirements retain their exact raw value and a scale refusal.
A missing default leaves only the affected requirement null, with a counted
refusal. Reward ordinals, independent explicit requirements, names and icons stay
reachable; no requirement is manufactured.

Database reads, failures, default reads and static track/subdoctrine projections
are cached per source/content binding. Reached mutable strings bypass those
caches. Before reading any content, the query builds folders, tracks and witnessed
grand/subdoctrine choices from its selected, prefix-clipped numeric window. A
missing content choice, database or catalogue retains this tree and all numeric
series with null names and counted refusals. A failed localisation catalogue does
not withhold authored membership or reward requirements from a known database.
Queries select only the requested country's series pages, check cancellation and
refuse oversized complete replies rather than truncating them.
The lead must register normal supplier/content query caching and gate dispatch.

The shared doctrine reader retains authored token spellings, exact icon keys and
grand-folder membership. Every grand choice is listed in registration order, not
just witnessed selections. Track ordinals use witnessed grands' authored lists;
with no witnessed grand they use the folder's authored choices. An ordinal whose
track identity differs between those choices remains a named ambiguity, never an
arbitrary selected label. Subdoctrine alternatives retain authored membership,
and composite reward localisation always uses the definition's spelling, even
before a subdoctrine has ever been observed. Witnessed choice ids keep their
recorded spelling so they still match the numeric `tokens` dictionary; unwitnessed
choices use their authored spelling. Repeated declarations preserve the shared
reader's existing object/list rules. Folder/track numeric ids and the `present`
and `completed|<ordinal>` series are unchanged.

Folder, grand and subdoctrine `icon` values are exact sprite keys for the shared
`sprite-artwork` route ([dashboard-base](../dashboard-base/README.md)), not image
URLs or aliases guessed from tokens. A missing or refused key is null with a
named, counted warning. The widget requests only the keys it draws.

Descriptions share the reached Doctrines benefit formatter. Without `hour`,
static fragments survive and mutable operands remain named refusals: history
must not silently freeze them at a branch-end country state. For a demanded
reward tooltip, request `doctrine-mastery` with `hour` and use its description
only at that hour. Setting `firstHour` and `lastHour` to the same hour keeps the
transient numeric window small. The shared frame cache reads the
scoped-text/trigger and doctrine inputs once for the request; the country is
resolved at that hour through Countries/CountryLinks. Gaps, missing operands,
unresolved country identity and hours beyond the accepted prefix retain static
fragments and numbers with counted refusals. Mutable results never enter the
content-only cache or leak between countries/hours. Commander contributions,
grants and authored costs remain the existing Doctrines detail product's scope.

## Validation

```powershell
cargo nextest run -p dashboard-doctrine-mastery
```

Content-reader tests explicitly initialise verified executable metadata from
`BATTLE_PLAN_GAME_DIR`. All game inputs are read-only; fixtures and prepared files are
synthetic and outside Git. Tests cover exact wide/signed values, empty versus
absent/unassigned/unknown slots, canonical aliases, token failure, content-failure
caching, publication/resume, read dates, gaps, prefix clipping, tag reuse, cold
reply shape, cancellation, mod folders/tracks and unequal reward requirements,
and 5,000 countries with a sparse selected-country reply. Recorded-structure tests cover
numeric-tree reachability without content, cached database failure, catalogue
failure with independent requirements, missing definitions and token spellings,
window-only track membership, a missing default without reward ordinal loss,
and unknown versus authored-empty rewards.
Layered fixtures cover
mod icons/grand choices, replace_path, unobserved mixed-case reward localisation,
and exact reached conditional descriptions across known/missing flags, another
country's scope, prefix clipping, repeat requests and static-cache reuse.

