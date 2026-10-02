# Special projects

The `special-projects` product covers one country over the branch. Live cards
show the native overall prototype progress and assigned scientist; History uses
the shared lane timeline, grouped with recorded country flags when comparing
countries. The header's Live / History choice is saved as `view`. The frame's
Live switch follows playback; off shows cards at the branch end and the whole
campaign timeline. Completed cards stay for `completedRetentionDays` (240 game
days by default).

The iteration row is isolated in [IterationRow.tsx](IterationRow.tsx).
Its text is the game's `ONGOING_PROJECT_BREAKTHROUGHS_NUMBER` localisation with
the native completed-history count (`141ED9730`). It has no denominator or dots.
Country specialization breakthrough points are not attributable to a project.

Starts say **by** the first observation: the native start writes no date event.
Recognized completion history gives an exact date; otherwise completion also
says **by**. Daily reads are dated with the shared reads helper. Missing reads,
recording gaps and changed country identities break observations. Progress is
stepped, never interpolated, and a future completion is hidden until observed.
Names and portraits use the recorded hour and content stack, with visible
fallbacks and diagnostics when unavailable. A missing table is distinct from a
valid empty set.

Completion clears the native program reference. When an earlier observation in
the same episode holds a scientist, completed views say **Last assigned** and
the tooltip dates that attribution; gaps do not supply an assignment. Scientist
portraits request native role 4, large size (`141F03DE0`), through the shared
character portrait resolver. Missing count localisation shows an explicit
fallback separately from missing history.

Right-click a card or lane (or use its context-menu key) to jump to the first
start observation or the completion date. The timeline supplies its own zoom,
hover, keyboard navigation and playhead. Live shows the first selected country;
History may compare the selected countries.

Native meaning, content-reader, gap, mod fixture and UI helper tests are offline.
The lead performs the browser comparison against diplo-1. Game display parity,
fresh-save oracle and capture smoke remain unverified under the no-game rule.

Paused projects are named. Cancelled projects leave Live and end History **by**
their cancellation observation, with muted bars. Matching episodes continue
across gaps without filling missing hours. The reply sends one count-to-text
table, rendered only by IterationRow. The shared ProgressMeter clamps negative
native progress to an empty fill while tooltips retain the recorded percentage.
