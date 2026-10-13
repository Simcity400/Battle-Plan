# Wars product

`wars` returns a whole branch's native war overview groups for one viewpoint.
The default follows recorded LocalPlayer, including observer choices;
`viewpointTag` selects another recorded country. Groups can differ by viewpoint
and overlap: this is not a global war census. Factions prune candidates but
factions and subjects do not add participants. Preserve native array order,
including retired occurrences and swap removals.

## Layout

| File | Responsibility |
|---|---|
| [../wars.rs](../wars.rs) | Product, branch/viewpoint cache, dated projection, app continuity and casualty membership shared with breakdowns. |
| [native.rs](native.rs) | Grouping, representative selection and indexed deduplication. |
| [naming.rs](naming.rs) | Recorded native name operands and content-resolved war names. |
| [labels.rs](labels.rs) | Historical country names for label measurement. |
| [tests.rs](tests.rs) | Synthetic grouping properties, continuity, naming, scale and reply-budget cases. |

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

## Names and cost

Names use recorded prefix/first-goal operands, recorded content and shared
country/state resolvers. Only a content key for the alliance variant reads the
viewpoint's alliance test, whose cached-subject operand exists only for the
recording client with scores captured; other wars take the pair key, else the
prefix, from any viewpoint. Missing or refused names retain their reason, without
an invented title. Stored text keeps the shared recording-language guard.
`countryNames` is only a catalogue for measuring historical labels; it never
fills a name, flag, roster or value at a missing hour.

Cache the most recently requested replies by branch, viewpoint/native selection
and content catalogue. Reuse unchanged structural inputs when only casualties
change. The Rust-only relation membership feeds casualty breakdowns without
another grouping algorithm. Counts
have no chosen ceiling; measure the complete reply against the shared
`MAX_PRODUCT_REPLY_BYTES` and refuse excess bytes without shortening history.
Indexed fast paths preserve the grouping properties checked in `tests.rs`; arbitrary
graphs can still require superlinear work and overlapping output.

Use the [dashboard checks](../../../README.md#check-a-change), including generated
types when wire types change. Keep synthetic coverage for native order/repeats,
splits/merges, identity and gap boundaries, missing names, 5,000-country graphs
and exact reply-budget refusals. Live integration follows the
[recorder workflow](../../../../recorder/README.md).
