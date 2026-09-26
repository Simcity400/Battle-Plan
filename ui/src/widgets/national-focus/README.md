# National focus

For the whole campaign, lists the country's completed focuses in order, ending with any focus still unfinished when the recording ended. In Live or at a pinned moment, a current focus card above the list shows the game's artwork, whether it is in progress, paused or continuous, the day it has reached and, like the game's active-goal bar, a progress line with the days left. With no focus it says so, with any saved progress. Click a completed row for the authored description and base duration. Live keeps the newest completion in view as it lands.

**Product:** `national-focus` (`crates/dashboard/src/products/national_focus.rs`), parameters `branchId` and `tag`. The reply holds the whole branch as disjoint segments, the recorded progress runs and artwork for every token; `focusAt` and `focusProgressAt` resolve the playback hour locally.

**Progress:** `progressRaw` is the engine's fixed5 days of work (the daily update adds the day's focus speed; idle days bank it). A recording showed +1.00 a day and completion at cost × 7 days. The line is progress over the base duration; days left use the last recorded daily gain for the same focus and are omitted until one day has been recorded, never estimated across a gap. Progress changes daily for every country, so it has its own memory budget in the branch scan: past it, progress stops being recorded from that hour with a warning, and the order history stays complete.

**Rules:** a newly observed completion is dated "By" its first observation, an upper bound. Entries present at the first readable snapshot show "Date not recorded". Gaps stay unavailable; a later observation never fills an earlier one. Repeated focuses keep separate ordinals.

**Remaining:** none open. Artwork and descriptions come from the recorded content; scripted names stay unavailable.
