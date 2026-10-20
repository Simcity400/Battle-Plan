# Air statistics

**What it shows:** For one country, ours against the enemy's per strategic region for one recorded air statistic (fighters lost, buildings bombed and the rest), in the game's current year window at the read behind the hour shown.

**Product(s) it reads:** [`air-statistics`](../../../../crates/dashboard-air-operations/README.md), a keyed family prepared from the daily AirStatistics reads. One request per country and statistic holds the whole branch; playback looks the hour up locally.

**Settings:** Statistic (also the dropdown above the chart; Fighters lost by default), Air theatres, Regions or Number of regions (10 most active by default), Values at bar ends and Ratio column.

**Notable behaviour:**

- Values are the game's current year window (twelve 30-day periods) at the supplying daily read, never summed across windows or differenced into losses. The tooltip dates the read.
- Each side's header names its own measure, so the ships pair keeps its two meanings (our ships bombed, enemy ships sunk).
- A region without a statistics record is absent, never drawn as zero; a recorded zero is drawn as zero. Hours outside the recorded reads say so.
- Regions are ordered by ours plus enemy. The ratio column is enemy per ours.
- Unprepared hours say they are still being prepared; Jump reaches the supplying read.

**Files:** [AirStatisticsWidget.tsx](AirStatisticsWidget.tsx) draws the butterfly; [airStatisticsData.ts](airStatisticsData.ts) validates the reply and looks pairs up; [useAirStatistics.ts](useAirStatistics.ts) requests the history and lists the settings' choices.

Shared framework conventions: [widgets](../README.md).
