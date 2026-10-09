# Native war overview evidence

This audit extends the war-object evidence in [cwar_relation.rs](../cwar_relation.rs).
Addresses refer to the supported executable identified by [build.rs](../build.rs).
The bodies and instructions were read through [hoi4re](../../../../tools/re/README.md).
These are the native rules implemented by the
[overview reader](overview.rs), [goal reader](../cwar_goal.rs) and
[wars product](../../../dashboard/src/products/wars/README.md).
Synthetic agreement does not establish live capture or displayed-name parity.

## The viewing country matters

`CDiplomacyStatus::BuildWarBlobs@140D27BB0` receives one country's diplomacy
status. `CWarOverView` calls it for the local playing country when positive,
otherwise the observer's selected country (`141898C90`, `14189CAD0`;
`CGameState+0x520/+0x524`). There is no global war-group census in these calls.
Different countries can produce different, overlapping groups from the same
relations. The resulting arrays have neither an identity nor a start or end.

The two arrays are **the seed country's side and the other country's side**.
The side builder does not read `first_was_instigator`. It does not intrinsically
label the arrays attackers and defenders. `BuildWarName` uses the selected
bilateral relation's instigator to order its naming operands. Orienting a
timeline's whole group by that representative's instigator needs to be stated
as the product's convention; it is not a property stored on a native war.

## Building and deduplicating sides

Use canonical country equality (`140BA60A0`, `140BA6240`), not raw-reference
equality alone. Native array order matters even though deduplication ignores it.

1. `140D27BB0` walks `CDiplomacyStatus+0x20`, count `+0x2C`, in native order.
   For every owned relation whose type word at `+8` is the war type, it adds
   `GetOther(owner)` to a temporary seed list. This loop does **not** filter the
   relation's retired byte. It calls `1419730E0` for each seed; that function is
   a thunk into `BuildWarBlobSides@1419730F0`.
2. `1419730F0` clears both arrays. It looks up the seed pair through the seed
   country's diplomacy matrix, target canonical slot, `CRelationStatus+0x2E8`.
   A null war pointer or nonzero `CRelation+0x49` returns two empty arrays.
   This test uses the retired byte, not inferred start/end-date inequalities.
3. `BuildWarBlobCandidateSide@1419728B0(A,B)` starts its result with A. It walks
   B's **owned** relations in native order and selects every war-type relation,
   including retired occurrences. For each other endpoint X, it skips A and
   appends X exactly when A's matrix war pointer toward X is null or retired.
   Thus the candidates are A and B's other war-relation counterparts who are
   not actively at war with A. Faction members and subjects are not independently
   enumerated or automatically added.
4. In that candidate array, every pair of nonseed candidates at indices
   `i < j` is checked for an active bilateral war. If they are at war, compare
   each candidate's faction pointer with A's nonnull faction pointer
   (`CDiplomacyStatus::SameFaction@140D2AC10`). When both match A's faction,
   mark both for removal; when neither matches, mark both; when exactly one
   matches, mark only the candidate that does not. The seed is never removed.
   These tests all inspect the original candidate array before removals.
   Remove unique marked indices in descending order by copying the last
   element into the removed slot and decrementing the count. This is not
   stable filtering and must not be replaced by `retain`.
5. `1419730F0` builds its first side with `(A,B)` and second with `(B,A)`.
   For every nonseed first-side participant and every nonseed second-side
   participant it tests for an active bilateral war. Any failed test marks
   the **second-side index** for removal. It then performs the same descending
   swap removals. The first-side removal array is initialized but never filled
   in this function. Instructions `1419732BB..14197348E` confirm that asymmetry;
   do not replace it with a symmetric clique or connected-component rule.
6. `140D27BB0` discards a pair if either side is empty. It compares it with
   each existing output pair: both corresponding lengths must agree, every
   first-side member must be present in the candidate first side, and likewise
   for the second side. Equality is canonical and membership ignores array
   order. Reversed sides are not compared as equal. The first matching output
   keeps its arrays; a distinct pair is appended. This deduplication is local
   to that call's viewing country.

The candidate-pair tests and cross-side tests are nested loops. Faithfully
transcribing the native algorithm does not establish a linear-in-input-rows
cost bound. Building only when inputs change, caching a branch reply, indexing
pair lookups and keeping all work off rendering are necessary, but do not by
themselves prove that bound for arbitrary modded relation graphs.

A faction change can change these groups without any war relation changing.
Cache invalidation therefore needs the owned-war order, active matrix pairs,
country links and faction membership. Naming has further hourly dependencies:
major bytes, strength scores, the local viewpoint, subject-pointer inputs, goal
operands and country/state names. A relation-only change detector is insufficient.
Overlapping groups also mean a continuity rule based on a shared bilateral
relation needs an explicit, deterministic policy for multiple matches; sharing
one relation is not proof that two simultaneous blobs are one war.

## Selecting the naming relation

`FindWarBlobRepresentativeRelation@1419734F0` takes a copy of an ordered side
pair and consumes its arrays. It does not simply select the oldest relation.

First it walks first-side order, then second-side order, looking up each active
cross-side war. The first active relation becomes the candidate. A later one
replaces it only for a **strictly smaller signed start hour** (`relation+0x20`).
Equal starts therefore retain the first array-order occurrence.

It reads the candidate's stored first and second countries, then their raw
`CCountry+0x145A` major bytes. If both bytes are nonzero, it returns that oldest
candidate. Otherwise it sorts each side stably, majors before minors and,
among equal major bytes, by descending signed strength score. The score is
the signed division-roster count (`CCountry+0x29C`, getter `1406C25B0`) plus
`CProductionStatus::CountWarOverviewFactories@140E548B0`, added with native
32-bit arithmetic. Equal scores keep the original side order. The insertion
sort `1419720E0` and the larger stable merge path `1419722A0`, `141971B00`,
`141971D30`, `141971660` use the same strict comparator.

The factory helper is **not a ship count** and is not the sum of the three
CountryFactories cached groups. Through `CProductionStatus+0x290` it obtains
the owning country. It walks that country's core-state pointer array at
`+0x4A8`, count `+0x4B4`, plus one terminal offsite pass. A nonnull state counts
only when its controller at `CState+0xCC` canonically equals that country.
For each entry in the state's building vector at `+0x170`, it follows
`CBuilding+0x1E0` to its template and adds the signed short level at `+0x40`
exactly once when the signed 32-bit sum of the template's naval, military,
general and rocket production words (`+0x2F8/+0x2FC/+0x300/+0x30C`) is positive.
The terminal pass walks `CCountry+0x1350`'s `CBuildingStatus+0x50` active vector.
A null core-state occurrence also takes that offsite path, so it must not
silently become a skipped state. The running total uses native 32-bit wrapping.
The template reader `1414A9220` maps rocket production to token `0x32B3`.

The representative search now aligns the arrays with the oldest candidate's
stored first/second countries, regardless of its instigator byte. A major
endpoint remains fixed to that original endpoint. A minor endpoint is instead
the current member of its sorted side. It starts both side cursors at zero and
returns the first active relation between the selected endpoints. When no
relation exists, it advances the first-side cursor if that member's score is
strictly greater than the second-side member's score; otherwise it advances
the second cursor. When one cursor runs out, it checks remaining members
against the exhausted side's last member, unless that endpoint was fixed as
a major. If the entire search fails, it returns the original oldest candidate.
Instructions `1419737A4..141973B67` confirm the cursor and major-pin branches.

## Naming operands and fallbacks

`CWarRelation::BuildWarName@140D0BCA0` reads these operands:

- The stored endpoint references and `first_was_instigator` byte at `+0x158`.
  Nonzero means the first endpoint is the instigator. All `FIRST` operands
  below refer to that instigator, not necessarily the blob's first side.
- The default prefix literal at VA `14294A860`, whose bytes are `WAR_NAME`.
  This is a native naming operand, not a catalogue of vanilla war names.
  Under the project's no-hard-coded-localisation-keys rule, record or otherwise
  expose this native operand; a product must not invent its own English format.
- The instigator's first goal reference, if its goal array is nonempty.
  `GetWarGoalsForCountry@140D0FD10` selects `relation+0x168` for the stored
  first endpoint and `+0x180` for the second. These are arrays of eight-byte
  handles, not pointers. Resolution is `14220A3D0`. A null handle leaves the
  default prefix. A resolved `CWarGoal+0x40` points to its `CWarGoalType`; a
  nonempty type string at `+0x2A8` replaces the prefix. The type reader
  `140A2C950` loads it under token `0x2C18` (`war_name`). Resolve that override
  from the recorded content stack's `common/wargoals`, not a vanilla list.
- The **raw endpoint tags** (`140BA5C20`), not cosmetic names, form
  `prefix_instigatorTag_defenderTag`.
- The local playing/observer-view country (`+0x520/+0x524`).
  `1406F3A90(local,endpoint,0,0)` returns true for self, otherwise rejects an
  active bilateral war, then accepts matching nonnull faction pointers or
  either orientation of the matrix's subject relation (`140D11030`,
  `140D11080`). The optional pact/guarantee branches are disabled by the two
  zero arguments. The subject tests inspect the cached relation's type and
  endpoints; they do not inspect its dates or retired byte.
- If the local country is allied to the instigator, or otherwise allied to
  the defender, the code constructs the extra variant
  `prefix_instigatorTag_defenderTag_defenderTag`. Both branches append the
  **defender tag**; instructions `140D0C065..140D0C150` confirm the same R13
  endpoint in both branches. It first uses that key when localisation contains
  it (`14222F900`), otherwise the unsuffixed pair key when present, otherwise
  the original prefix. Do not correct this asymmetry in the app.
- String arguments `FIRST` and `SECOND`: the endpoints' current adjectives
  (`140BA5560`, `CCountry+0x30`). `FIRST_COUNTRY` and `SECOND_COUNTRY`: their
  current names (`140222400`, `CCountry+0x10`). Reuse the shared country-name
  resolver for names and adjectives as of this hour.
- Optional arguments from the same instigator first goal: `STATE` is the
  first state's current display name (`CWarGoal+0x48` pointer array,
  count `+0x54`, `CState::display_name@1409CC380`), or an empty literal when
  that array is empty. When the goal's country array at `+0x60`, count `+0x6C`,
  is nonempty, its first country's current name and adjective supply `COUNTRY`
  and `COUNTRY_ADJ`. A missing first goal does not insert those arguments.
- `ORDER` is the empty literal on the initial iteration. The apparent ordinal
  loop has a counter local to this invocation, initially one. Its collision
  call at `140D0D4A7` reaches `ReturnZero@14011CD10`, whose complete body is
  `XOR AL, AL; RET` (`14011CD10..14011CD12`). Consequently this supported
  build returns after that first formatting pass. **There is no persistent
  war-order counter to capture, and no generated "Second" ordinal here.**
  The unreachable later iterations format the number, a localised ordinal
  suffix and a space. This does not license inferring an ordinal from history.

Final formatting is the plain game localiser `142230F20`, reached directly
by the overview text setter. It does not evaluate bracketed scripted text.
The STATE branch copies `CState::display_name` directly into argument kind 1
(`140D032C0`). The template's `+0x20` cache was already localised by
`140AAF580`; replay that plain localisation without `ProcessString`.
Content keys, type overrides, substitution and country/state text must retain
the existing installed-content localisation semantics and explicit refusals.

## Coverage audit before this capture

The pre-capture rows and cadence in [replay](../../../replay/src/rows.rs) and
[extended_rows.rs](../../../replay/src/extended_rows.rs) establish the following.
The following retained audit explains why the new operands were necessary;
it is not a current table or version catalogue.

| Native operand | Current recording |
|---|---|
| Bilateral endpoints, start/end, cancel, instigator, four casualty integers | WarRelations, hourly; native reader excludes retired relations and emits each active pair once. |
| Ordered owned war occurrences, including retired ones | Missing. WarRelations key order is not either country's owned order. RelationTuples describes a different, serializable roster and omits war rows. |
| Matrix war presence and retired predicate | Active WarRelations supplies active pairs; retired owned counterparts and their raw retired bytes are missing. Do not substitute date-derived `war_ongoing` for the native matrix test. |
| Faction equality | Factions and FactionMembers, hourly. Resolve through CountryLinks. |
| Subject equality used for naming | Subjects, hourly, supplies active subject pairs; the naming predicate's cached subject-pointer behavior has to be authenticated against this coverage before treating an absent row as false. |
| Local viewing country | LocalPlayer, hourly. An unavailable row is not an observer. |
| Major byte and division-roster count | Missing. Divisions is a census, not proof of the native raw roster count. |
| Core-state factory score | Missing at the required freshness. StateCores is sliced; CountryFactories counts controlled states by individual production flags, whereas this helper counts controlled **core** states with one signed sum predicate, including rocket production. |
| Ordered relation goal handles and goal type/actor/recipient | Missing. Relation serializer `140D19640` writes first/second goal handles in native order. |
| First goal's first state and country-list operands | Missing. Type and recipient alone cannot supply `STATE`, `COUNTRY`, `COUNTRY_ADJ`. |
| Current country names/adjectives and state display names | Existing shared installed-content resolvers and hourly country/state operands. Preserve recorded-language limits on stored custom text. |
| Native default naming prefix | Not a recorded war-naming operand. Do not replace it with a product-authored localisation key. |
| Capitulated flag and dated capitulation | Capitulation, hourly; no additional capture needed. Event attribution requires identity at the event's hour, as country_groups already enforces. |
| Persistent identity, native grouped start/end and participant join/leave dates | The game does not store them. Observation runs and continuity are app rules. |
| Order counter | No missing capture: the native collision check always returns false. |

`CWarGoal`'s authenticated primary vtable is VA `1429C9468`; its handle is at
`+8`. The embedded goal data starts at `+0x38`: actor `+0x38`, recipient
`+0x3C`, type pointer `+0x40`, state array `+0x48`, country array `+0x60`.
`CWarGoalType`'s primary vtable is VA `1429258A0`, type token `+8`.
Constructors `1415CB8A0`, `1415CBDD0`, `1415CBE80` and serializer
`CWarGoal::Serialize@1415CE720 -> SerializeData@1415CE460` establish the fields.
The latter writes actor, recipient, ordered state IDs, ordered country tags,
another country list at goal `+0x78`, and the type token. That last country
list is not read by `BuildWarName`, so it is not a new naming-capture requirement.

The relation serializer writes `first_wargoals`/`second_wargoals` handle lists,
not the pointed-to goal's whole data. An oracle must join those references to
the serialized goal objects before comparing type and naming operands, and
preserve list order and repeated occurrences. Empty lists and unreadable lists
must remain distinct. The save does not serialize the retired byte, the
diplomacy-owned occurrence order, or a computed blob/representative/name.
Those need named oracle limitations and the separate, read-only, two-war
overview comparison. No live save, timing or screen comparison was run in
this audit; the game was not launched.

The requested capture run sheet needs the following limitations to be kept
separate from actual differences:

- "The text save does not serialize war relation retired bytes."
- "The text save does not serialize diplomacy-owned war occurrence order."
- "The game does not store grouped war identities, group start/end hours or participant join/leave hours."
- "The text save does not serialize computed war groups, representative selection or displayed war names."
- "Stored custom country/state text is exact only in the recording client's language."

## Capture and oracle coverage

The new reader captures the missing owned occurrences, all ordered goal handles,
and each resolved goal's type/actor/recipient, list lengths and first state/country.
The remaining entries in those goal lists are not naming operands and are not
captured. Representative inputs and the recording viewpoint's cached subject
predicate are hourly; the native prefix is static. Table IDs and cadence live
only in replay code. The [oracle](../../../recorder/src/oracle/wars.rs) compares
ordered references, joins full goal objects and checks saved naming operands.
`CCountry::Write@14070C580` writes `major` (token 0x2BE9) only for a nonzero
byte at +0x145A, so the oracle verifies its boolean projection and names any
noncanonical raw byte separately. Raw division-roster count, controlled-core/
offsite factory score and cached subject predicate lack proven exact save
counterparts and are explicit limitations. The default prefix is executable
data. Live cost, fresh-save agreement and two-war displayed names remain
unverified until the run sheet is exercised by the live agent.
