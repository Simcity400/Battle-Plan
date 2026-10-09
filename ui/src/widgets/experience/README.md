# Experience

**What it shows:** Army, navy and air experience per country as lines: banked amounts, gross daily gains or net changes per day.

**Product(s) it reads:** [`country-experience`](../../../../crates/dashboard/src/products/country_experience.rs) supplies exact raw amounts, scales, read dates and unknown reasons; [`country-groups`](../../../../crates/dashboard/src/products/country_groups.rs) supplies groups and capitulations.

**Settings:** Banked and all three branches by default; one country or comparisons/smart groups, end labels, playback window, Show after the playhead and value-axis controls; Campaign, Follow, Pin and Range. No aggregate view.

**Notable behaviour:**

- Banked values hold within recorded coverage. Daily gain requires every preceding hourly interval for the same native instance; a partial day is not recorded. Change per day needs an actual bank read exactly a day earlier, includes spending/caps and never substitutes daily accumulators or a carried read.
- Manual tags follow their native holders through aliases. Renames preserve valid comparisons; reused holders, ambiguous identity, gain-instance changes and gaps break continuity. Smart-group membership stays literal and dated.
- Exact signed raws are rescaled using the product's scale. One country uses stable branch colours; comparisons use country colours and branch dashes. Labels/hover values retain their own read dates and proven aliases, never metadata fallback names.
- Capitulation badges do not end lines; their names/flags use the event hour, independently of numeric hovers. End labels use each line's actual last drawn hour. Missing marker history is separate from missing experience.
- The shared TimeChart owns keyboard, zoom and Jump; no earned/spent cause breakdown is built.

**Files:** [ExperienceWidget.tsx](ExperienceWidget.tsx) renders; [experienceData.ts](experienceData.ts) validates and projects exact reads, with adjacent tests. `experience.css`, `definition.ts` and `index.ts` own styling, settings and registration.

Shared framework conventions: [widgets](../README.md).
