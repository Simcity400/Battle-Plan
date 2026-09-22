# Research

One column per research slot: the assignment at the selected hour with its estimated days left, then the completed technologies in order. Expanding a row shows how long the research took against its base cost, the timeline of rate periods and the modifier totals for the selected period.

**Product:** `research` (`crates/dashboard/src/products/research.rs`), parameters `branchId` and `tag`. The reply holds the branch's slot histories, states and progress plus artwork; `researchAt` and `researchStateAt` resolve the playback hour locally.

**Rules:** dates are observation bounds; an outcome is never shown before it was observed. Slots that appear later start offset below their neighbours. Rewinding hides later slots, assignments and outcomes. Fixed-point operands stay exact as decimal strings; floating point is only for layout.

**Remaining:** fresh-save agreement for the hourly research tables is not yet validated by an oracle run.
