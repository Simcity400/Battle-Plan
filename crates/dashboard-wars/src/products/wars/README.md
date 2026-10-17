# Wars product

Prepared `wars` queries return a branch's native war overview groups for one viewpoint.
The default follows recorded LocalPlayer, including observer choices;
`viewpointTag` selects another recorded country. Groups can differ by viewpoint
and overlap: this is not a global war census. Factions prune candidates but
factions and subjects do not add participants. Preserve native array order,
including retired occurrences and swap removals.

## Layout

| File | Responsibility |
|---|---|
| [../wars.rs](../wars.rs) | Product metadata, dated projection, app continuity and casualty membership; the recording loader is a test oracle and the shared branch/viewpoint cache also serves casualty breakdowns. |
| [prepared.rs](prepared.rs) | One branch-global change observer, shared relation casualty series and reached-hour reply assembly. |
| [block.rs](block.rs) | The wars prepared on their own: the fields read, the block extract and fold, and each branch's observer and published walk. |
| [published.rs](published.rs) | The recorded viewpoint's walk published while preparing: closed wars, resume points and the tail. |
| [walks.rs](walks.rs) | Each requested viewpoint's walk over the change inputs, kept on the source with resume points; the published walk's reads; the change input row format. |
| [native.rs](native.rs) | Grouping, representative selection and indexed deduplication. |
| [naming.rs](naming.rs) | Recorded native name operands and content-resolved war names. |
| [labels.rs](labels.rs) | Historical country names for label measurement. |
| [tests.rs](tests.rs) | Synthetic grouping properties, continuity, naming, kept walks, scale and reply-budget cases. |

Display and settings belong to the
[Wars widget](../../../../../ui/src/widgets/wars/README.md).

## Identity and dates

The game has no grouped-war identity. In native seed order, each new group
continues the still-unmatched previous group sharing the most uninterrupted
bilateral relations; ties choose the lowest branch-local app ID. Matching is
one-to-one over consecutive observed hours under the same viewpoint. Relations
include native endpoint identities, recorded tags and start hour. Absence,
redeclaration, unreadable inputs, a missing hour or viewpoint change breaks
continuity. A replacement country cannot inherit a war solely through its tag.

First/last hours are observations. `startedBy`/`endedBy` and participant
`joinedBy`/`leftBy` mark uncertain boundaries: display them as "by" and leave
unobserved spans blank. The representative's instigator orients attackers and
defenders as an app convention; missing selection inputs leave orientation
unknown while preserving the native viewpoint/opponent arrays. Names and sides
come from the observation covering the requested hour.

`available` requires at least one complete or truncated overview operand view,
including a complete empty table; it does not depend on viewpoint or naming
success. No such view means "Not in this recording". Present but unreadable
hours remain "Not recorded" in `unobserved`. `viewpoints` includes successful
peace observations; switches, unreadable hours and gaps end its dated runs.
Captured overview viewpoints must agree with LocalPlayer; foreign views cannot
use scores or cached-subject naming predicates outside the captured reach.

Casualties have independent compact dated runs, summing distinct active
cross-side bilateral relations in their stored endpoint order. Keep exact signed
integer strings and separate known/unknown columns; no run crosses a gap.
Overlapping groups can share losses, so never sum timelines into a campaign
total. Capitulations use recorded flag/hour evidence and require a continuously
observed identity whose participant run contains the event; unresolved identities
warn, and missing capitulation coverage has its own spans.

## Prepared queries

The pipeline observes native inputs once for the branch. Shared events retain
changed topology/naming tables and capitulation evidence. Each change input is
a raw event row, not JSON: the number of changed tables and their numbers as
little-endian u16s, then the changed tables in the prepared decoder checkpoint
encoding, compressed. The inputs and the held tables leave out the clock,
whose rows no reply reads; an input at a capitulation date with no table
changed lists none.
Semantic name operands ignore unrelated hourly balances; casualty-only hours add
no input event. One small catalogue keeps relation-lifetime keys, their first
supplying read and the latest inputs for resume. New keys are numbered in
recorded WarRelations row order, so equal inputs publish equal catalogues. Four shared series hold exact
counters in canonical endpoint order, with unknown spans preserved.

The wars are prepared on their own ([block.rs](block.rs)), before the hourly
walk, and their query waits only for their own coverage. A block is read at
the hours a field the observer compares changes: every field of the war
relations (whose counters are published), the overview, goal, name-rule,
membership, player, dictionary and reserved-naming tables, the campaign and
the country links; the countries' slots and tags; the factions' keys; the
capitulation flags and dates; the naming operands `same_name_rows` compares;
and the states' names. The next recorded capitulation date is the extract's
deadline, observed on the last hour's rows. Casualty counters move every hour
of a war, so the per-hour work runs in the extract, on the pre-pass workers:
it runs the observer on the block alone from the block's first hour,
observing each kept hour and advancing each span of hours between (each hour
authenticated, the held inputs moved to its last; the fold authenticates the
span in constant time), and records the calls it makes on its series after
the first hour, the change
inputs it publishes and the relation lifetimes it numbers. A block with a
truncated input that is compared by some of its fields only, or whose quiet
hours do not continue the hour observed before, is observed hour by hour.

The family's own thread folds the blocks in order and observes no hour but
each block's first on the carried observer. After that hour both observers
hold inputs equal in every compared field, a join of the same relations and
the same last value of each of their counters; they differ only in the
numbering of relation lifetimes and the count of change inputs. The fold
numbers the extract's lifetimes into the carried catalogue (a lifetime the
extract met first takes the next number, with its first read, as continued
observation numbers it), makes the recorded calls again under that
numbering, observes each change-input ordinal on the carried count, takes
each input view the extract replaced after the first hour, its join and its
next capitulation date, and walks the published inputs with the recorded
viewpoint's walk after the hour observed before each. When the carried
observer does not hold that hour's inputs and a join as the extract's does,
or the extract's observer had no join and took its relations from its open
counters, the rest of the block is observed hour by hour.

The shared frame fact guards native relation normalisation and semantic input
comparisons. Unchanged hours advance the accumulator frontier; balance-only
country changes preserve resolved operands. Changed casualties refresh the held
native join and observe only changed, new or removed counters. Casualty-only changes compare
topology without copying and zeroing the relation rows. The four counters
reuse each relation's key and canonical endpoint order until the join changes.
Naming-only changes publish their event without revisiting unchanged relation counters. A stored future
capitulation date still becomes usable when the accepted clock reaches it.

A query applies its viewpoint or LocalPlayer through the same native observation,
continuity and reply builder as the branch scan, decoding one change input at a
time. The walk is whole-branch and depends on the viewpoint, the native-holder
selection and the content catalogue, so the source keeps it per such key
(`walks.rs`): the four most recently used walks, each with its state after
every published input and resumable copies every interval of inputs from the
branch start. A copy keeps what continuing the walk reads: each open war's
last observation and loss run and its open participant and relation runs,
the last dated run of each kind, the held inputs and the decoded tables,
which copies share; wars no longer open keep only their app ID. Its size
follows the wars open at it, not how long they lasted. The interval starts at one input and doubles while a
walk holds more than 1,024 copies or more than an estimated 64 MiB of them. The
first query for a key walks every input once. A later one walks only inputs
published since, extending the last input's coverage, which grows with
counter-only appends; it hydrates each requested hour from the nearest copy,
fewer than one interval of inputs before the hour's input. A store that no
longer holds the walk's last input starts it again; the catalogue is held and
matched by identity. Replies equal the walk from the branch start. Casualty counters read
contiguous requested `reads: [{hour}]` windows through the series reader, keeping
only reached values and preserving canonical run bounds across commits. Open
counter runs come from the accumulator. Two adjacent runs prove the earlier
bounds; a run count equal to elapsed hours proves hourly closed runs. One shared
`values_at` call per reached hour supplies all missing amounts, retaining one
amount per counter. Other history uses the requested windows once per key,
including when overlapping groups share that counter. Replies keep the ordinary
shape. The widget requests its displayed and hovered hours
independently of its structural timeline.

The default query's walk (the recorded LocalPlayer viewpoint, no native-holder
selection, the store's content catalogue) is published while preparing
([published.rs](published.rs)): the wars' own thread walks each change input
the fold publishes, in input order. A war that leaves the open groups no longer changes before the
reply is finished, so it is published once, settled as finishing settles it but
without capitulations, in the next flush's range of closed wars
(`wars-walk-closed-{branch}`). Every 128 inputs a resumable copy and the decoded
inputs are published as a point (`wars-walk-points-{branch}`); the copy holds
only the wars open at it (each as a query's copy keeps it) and the number of
wars, so its size follows the wars open at it, and the points waiting for the
next flush count toward the batch's bytes. Each flush
rewrites the tail (`wars-walk-{branch}`): the whole walk but its closed wars,
which keep their place and app ID only, with the decoded inputs and the last
hour observed, which the next input's coverage extends through. The held
inputs are rebuilt from the decoded inputs, and label operands are read again
by the next input. A default query reads the tail and puts the closed wars back
in place, attaches capitulations as finishing does, and hydrates each requested
hour from the nearest point, walking fewer than 128 inputs; the last input's
hour walks none. A store whose tail does not reach its last input, or whose
branch starts at another hour (prepared hour by hour, a restored observer
without a walk), walks the inputs as other viewpoints do.

`prepared::projection` returns the typed whole-branch `WarsReply`, including
Rust relation memberships and native lifetime keys, dated names, app IDs,
observed bounds, availability and warnings. A selected native-holder set uses
the same dated holder lookup as the branch scan, follows its aliases and never
falls back to LocalPlayer or a replacement holder. Exact casualty hours affect
only numeric hydration; earlier memberships and names remain in the projection.

## Names and cost

Names use recorded prefix/first-goal operands, recorded content and shared
country/state resolvers. Only a content key for the alliance variant reads the
viewpoint's alliance test, whose cached-subject operand exists only for the
recording client with scores captured; other wars take the pair key, else the
prefix, from any viewpoint. Missing or refused names retain their reason, without
an invented title. Stored text keeps the shared recording-language guard.
`countryNames` is only a catalogue for measuring historical labels; it never
fills a name, flag, roster or value at a missing hour.

The shared casualty-breakdown scan caches the most recently requested replies by
branch, viewpoint/native selection and content catalogue. Reuse unchanged structural inputs when only casualties
change. The Rust-only relation membership feeds casualty breakdowns without
another grouping algorithm. Counts
have no chosen ceiling; measure the complete reply against the shared
`MAX_PRODUCT_REPLY_BYTES` and refuse excess bytes without shortening history.
Indexed fast paths preserve the grouping properties checked in `tests.rs`; arbitrary
graphs can still require superlinear work and overlapping output.

Use the [dashboard checks](../../../../dashboard/README.md#check-a-change), including generated
types when wire types change. Keep synthetic coverage for native order/repeats,
splits/merges, identity and gap boundaries, missing names, 5,000-country graphs
and exact reply-budget refusals. Live integration follows the
[recorder workflow](../../../../recorder/README.md).
