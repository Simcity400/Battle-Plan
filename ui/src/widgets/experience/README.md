# Experience

**What it shows:** Army, navy and air experience per country as lines: banked amounts, gross daily gains or net changes per day.

**Product(s) it reads:** Prepared series supply width-bounded chart geometry and exact reached raw amounts, scales, supplying-read dates and unknown reasons. Before preparation covers the view, banked experience is sampled from the recording: each block of a long view at its first daily read, drawn faded and dashed under an **Approximate** key, and a short view exactly. All three banks share each block?s cached first-read observation. Reached hours and short exact windows reuse whole-block observations, clipped after preparation advances. Daily gain and change per day wait for preparation. [`country-experience`](../../../../crates/dashboard-base/src/products/country_experience.rs) supplies the history reference; [`country-groups`](../../../../crates/dashboard-base/src/products/country_groups.rs) supplies groups and capitulations.

**Settings:** Banked and all three branches by default; one country or comparisons/smart groups, end labels, playback window, Show after the playhead and value-axis controls; Campaign, Follow, Pin and Range. No aggregate view.

**Notable behaviour:**

- Banked values hold within recorded coverage. Daily gain requires every preceding hourly interval for the same native instance; a partial day is not recorded. Change per day needs an actual bank read exactly a day earlier, includes spending/caps and never substitutes daily accumulators or a carried read.
- Manual tags follow their native holders through aliases. Renames preserve valid comparisons; reused holders, ambiguous identity, gain-instance changes and gaps break continuity. Smart-group membership stays literal and dated. If the shared identity catalogue fails, manual choices use the history product and keep its independent missing-marker warning.
- Exact signed raws are rescaled using the product's scale. One country uses stable branch colours; comparisons use country colours and branch dashes. Labels/hover values retain their own read dates and proven aliases, never metadata fallback names.
- Capitulation badges do not end lines; their names/flags use the event hour, independently of numeric hovers. End labels use each line's actual last drawn hour. Missing marker history is separate from missing experience.
- The shared TimeChart owns keyboard, zoom and Jump; no earned/spent cause breakdown is built.

**Files:** [preparedExperience.ts](preparedExperience.ts) selects native holders and reads prepared windows; the shared scalar parity hook exposes "Experience chart values". [ExperienceWidget.tsx](ExperienceWidget.tsx) renders; [experienceData.ts](experienceData.ts) validates and projects exact reads, with adjacent tests. `experience.css`, `definition.ts` and `index.ts` own styling, settings and registration.

Shared framework conventions: [widgets](../README.md).
