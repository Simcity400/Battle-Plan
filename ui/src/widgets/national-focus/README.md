# National focus

Shows a country's completed national focuses in order, then the focus in progress or the continuous focus. Click a row for the authored description and base duration.

**Product:** `national-focus` (`crates/dashboard/src/products/national_focus.rs`), parameters `branchId` and `tag`. The reply holds the whole branch as disjoint segments plus artwork for every token; `focusAt` picks the segment for the playback hour locally.

**Rules:** a newly observed completion is dated "By" its first observation, an upper bound. Entries present at the first readable snapshot show "Date not recorded". Gaps stay unavailable; a later observation never fills an earlier one. Repeated focuses keep separate ordinals.

**Remaining:** none open. Artwork and descriptions come from the recorded content; scripted names stay unavailable.
