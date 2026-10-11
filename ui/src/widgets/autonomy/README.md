# Autonomy

**What it shows:** Each subject's autonomy level, score and move-up/down thresholds on a lane timeline.

**Product(s) it reads:** [`autonomy`](../../../../crates/dashboard/src/products/autonomy.rs) supplies subject runs, levels and coverage; [`country-groups`](../../../../crates/dashboard/src/products/country_groups.rs) supplies native holder and smart-group histories.

**Settings:** Chosen countries are overlords by default, or subjects; manual countries and smart groups; Grow with playback; Campaign, Follow, Pin and Range.

**Notable behaviour:**

- Levels are ordered/named through recorded content, with threshold-based ordering when definitions are unavailable. Missing localized titles show Level name unavailable; recorded keys remain available in hover details. Palette colours follow level order and can repeat. Scores beyond thresholds draw at the edge; tooltips retain exact amounts. The relation's autonomy ratio is not interpreted.
- A known complete absence says **Not a subject here**. Missing tables, unknown native endpoints or unrecorded membership cannot prove absence. Unknown overlords retain known subject values but cannot pass an overlord filter.
- Manual tags select their native holders and aliases without selecting unrelated alias holders. Hourly groups clip at each endpoint's membership date; End/Ever choices keep their own counted dates.
- Gaps break bands, scores and thresholds. Level-change dots require adjacent recorded hours. Names/flags stay within native holder and readable spans; missing facts retain their reasons. Playheads outside the branch are unavailable.
- The shared timeline owns zoom, preserved growth, keyboard stops and Jump; score and threshold changes remain navigation stops even within one level.

**Files:** [AutonomyWidget.tsx](AutonomyWidget.tsx) renders the [lane timeline](../../charts/timeline/README.md); [autonomyData.ts](autonomyData.ts) validates, selects and clips subject runs, with adjacent tests. `autonomy.css`, `definition.ts` and `index.ts` own styling, settings and registration.

Shared framework conventions: [widgets](../README.md).
