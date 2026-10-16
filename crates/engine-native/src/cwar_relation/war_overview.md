# War overview operands

The [overview reader](overview.rs) records ordered bilateral relation occurrences,
country comparison inputs and goal references. The [goal reader](../cwar_goal.rs)
and [relation-goal reader](../cwar_goal/relation.rs) retain the goal operands used
for naming. The [wars product](../../../dashboard-wars/src/products/wars/README.md)
consumes recorded operands and content resolved from the recording's content stack.
This engine reader does not construct the temporary overview groups.

## Viewpoint and identity

An overview uses the playing country when one is selected, otherwise the observer's
selected country. An unavailable viewpoint is not an observer selection. Different
viewpoints can produce different, overlapping groups from the same bilateral relations.

Each temporary group has an ordered side for its seed country and an ordered side
for its counterpart. These sides have no intrinsic attacker/defender labels. The
representative bilateral relation supplies the instigator used for naming; orienting
the whole group around that instigator is a product convention.

The game stores bilateral relations, not persistent grouped war identities. The
temporary arrays have no stored group name, start/end hours or participant join/leave
dates. Recordings can describe observation runs and continuity, but those are app
rules. A shared relation alone cannot prove continuity when simultaneous groups overlap.

## Recorded relation and country operands

| Operand | Meaning and availability |
|---|---|
| Active bilateral relation | Stored endpoints, dates, cancel state, instigator and four signed casualty values. Active relations are emitted once from the first endpoint after reciprocal ownership validation. |
| Owned war occurrence | Owner, original position in the complete diplomacy vector, endpoints, raw retired byte and both goal-list lengths. Retired occurrences and repeated occurrences remain present. |
| Matrix war predicate | Uses the selected relation pointer and retired byte. Dates do not substitute for that predicate. |
| Country comparison inputs | Raw major byte, signed division-roster count and signed controlled-core/offsite factory score. A captured division census does not replace the stored roster count. |
| Viewpoint subject predicate | Cached subject relation toward the recorded viewpoint. Its type and endpoints matter; dates and retired state are deliberately unread. The active subject roster does not supply an equivalent absence test. |
| Faction membership and country aliases | Resolve through the same hour's country links and faction observations. Raw-reference equality alone does not establish country equality. |
| Local viewpoint | Recorded playing or observer-selected country for this hour, with its own availability. |

Country rows retain an explicit availability flag for strength and subject operands.
A zero flag means those fields were not read; their stored zeroes do not assert a
zero score or false predicate. Major bytes are retained independently.

Ordered owned occurrences differ from the active bilateral relation table and from
the serializable relation roster. Neither filtered row order can reconstruct the
complete owned order. Group dependencies include owned order, active matrix pairs,
country aliases and faction membership; a relation-only change detector is insufficient.

The factory score counts qualifying buildings in controlled core-state occurrences
and the terminal offsite pass. A null core-state occurrence also selects the offsite
pass. It uses signed production-category sums and wrapping accumulation, including
rocket production. The cached factory groups and a ship census are different operands.

## Goal and naming operands

Both endpoint goal lists retain complete handles in stored order, including repeats.
Each resolved goal records its type token, actor, recipient, state-list length and
first state, and country-list length and first country. Later members are not naming
operands and are not captured by this reader. Null or stale handles remain explicit;
a failed read refuses the affected table instead of producing an empty list.

Naming selects a representative bilateral relation from the ordered groups. Equal
start hours preserve occurrence order; major status and signed strength comparisons
can change the selected relation. The instigator's first goal can supply a content
override, while the default naming prefix is read from the supported installation.
No built-in war-name catalogue is included.

The selected endpoints supply raw tags, current names and adjectives. The first
goal can supply its first state's display name and its first country's name and
adjective. Missing goals, empty lists and unavailable operands stay distinct.
The viewpoint's alliance and cached-subject tests can select additional naming
variants; their endpoint ordering is preserved. There is no stored ordinal counter
to capture or infer from history.

Naming uses plain localisation with substitution, including state display text;
it does not evaluate bracketed scripted text. Resolve overrides, templates and
country/state text through the recorded content stack and the shared installed-content
resolvers. Preserve explicit refusals and the recording language of stored custom text.

## Acquisition and comparison

Worker storage grows from original acquired headers. Header fingerprints reuse
discovery only: pointer pages, retired bytes, goal lists and score operands are
checked freshly. The type-pointer gather keeps its bounded batch scratch in its
own frame, separate from the walker's owned pointer pages and type bytes.
Full refreshes and failures invalidate retained discovery. Native pointers stay
within the acquisition; they never become replay identities.

The [war oracle](../../../recorder-oracle-wars/src/wars.rs) compares ordered references,
joins serialized goal objects and checks saved naming operands. It preserves list
order, repeats and the distinction between an authenticated empty list and a failed
list. Saves serialize the boolean projection of major status; noncanonical raw
major bytes have a separate limitation.

Save comparison cannot establish every overview operand:

- Retired relation bytes and diplomacy-owned occurrence order are not serialized.
- Division-roster count, controlled-core/offsite factory score and the cached subject
  predicate have no exact save counterpart established by these readers.
- Temporary groups, representative selection and displayed names are not serialized.
- Group identities and participant join/leave dates do not exist as stored fields.
- Stored custom country/state text is exact in the recording client's language.
- The default naming prefix is installed executable data.

These availability limits are separate from actual save differences. Table IDs,
cadence and encoding are defined in replay code; the engine documents the acquired
meaning and the independent failure boundaries.
