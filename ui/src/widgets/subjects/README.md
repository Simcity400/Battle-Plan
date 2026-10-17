# Subjects

**What it shows:** An overlord/subject tree on a lane timeline, with relation spans, subject levels and overlord bands sized by subject count.

**Prepared data it reads:** [`subjects`](../../../../crates/dashboard-subjects/src/products/subjects.rs) supplies relation/level runs and coverage; [`country-groups`](../../../../crates/dashboard-base/src/products/country_groups.rs) supplies native endpoint identities and selection histories.

**Settings:** All countries by default, or chosen native countries' relevant trees through manual/smart groups; overlord/subject ordering, former subjects, level names/legend and Grow with playback; Campaign, Follow, Pin and Range.

The widget reads the prepared branch reply through shared loading and coverage
polling. Its relation/level observer and content reply builder also serve the
recording product used for exact comparisons.

**Notable behaviour:**

- Relation start is the game's recorded start, including before recording. Same endpoints/start across a gap remain one relation with a gap; a different start is another relation. A country may be both subject and overlord.
- Current level is the subject's Autonomy state, not the relation's initial Subjects state. Content orders/names levels; missing names show Autonomy level name unavailable, and palette overflow shares colour without hiding levels. Other levels lists the folded names on hover.
- Both endpoints and lanes key on native holders. Manual roots follow their holders/aliases; replacements using a tag get separate lanes. Smart groups retain hour/end/ever dates. Missing identity/coverage makes counts unknown, never proof of no subjects.
- Names/flags use proved aliases at shown/hovered dates, bounded by holder and readable spans. Pending/refused flags retain reasons. An unrecorded hour says **Not recorded**, not an ended relation.
- Live reveals reached lanes and grows bars; the shared timeline owns zoom, keyboard changes, complete tooltips and Jump. A changed overlord appears in the corresponding separate tree.
- While preparation runs, the prepared prefix is exact and each later block shows the relations and levels of its keyframe, drawn approximate (faded, dashed) and named in the key line; a relation's start stays the game's recorded start, and an end inside those hours reads "around" its date. Hours no keyframe answered are **Still being prepared**: a light band, unknown rather than an ending. The exact history replaces both as preparation reaches them. Per-block relation/level observations are cached across re-asks, so a moving prefix does not resample the same keyframes.

**Files:** [SubjectsWidget.tsx](SubjectsWidget.tsx) renders the [lane timeline](../../charts/timeline/README.md); [subjectsData.ts](subjectsData.ts) validates/shapes native trees and relation bounds, with adjacent tests. `subjects.css`, `definition.ts` and `index.ts` own styling, settings and registration.

Shared framework conventions: [widgets](../README.md).
