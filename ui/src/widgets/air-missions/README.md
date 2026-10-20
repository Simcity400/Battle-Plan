# Air missions

**What it shows:** Where one country's air wings are assigned at the hour shown: per strategic region, the planes on each mission (a column per mission kind the recording has) and the region's wings counted once.

**Product(s) it reads:** [`air-missions`](../../../../crates/dashboard-air-operations/README.md), a keyed family prepared from the hourly regional mission registrations and wings. One request per country holds the branch's history; playback looks the hour up locally. A region's wings are a demand-loaded request at the drill's hour.

**Settings:** Follow or Pin; Air theatres, Regions and Missions shown; the shared table settings (cell bars, density, When narrow, sort; Wings descending by default); where the breakdown opens.

**Notable behaviour:**

- Registrations belong to the wing pool's owner; a wing on several missions counts its planes in each mission's column but once in Wings, so mission columns are not summed into a total. A registered mission with no planes shows 0.
- Missions and regions come from the recording and its content stack; a missing name says so and is counted.
- Each mission column's head shows the game's mission icon: the frame of the mission strip the game's own map entry uses, through the content stack, drawn by the shared sprite route. Without one the head shows the name only.
- Mission columns are the lowest priority: they hide first as the widget narrows and their values move into the row tooltip.
- Clicking a region opens its wings (aircraft, planes, missions and efficiency where its operands are recorded), following playback or pinned. Jump reaches the read behind the hour.
- No idle-plane, sortie or averaged detection figures: the recording does not support them.

**Files:** [AirMissionsWidget.tsx](AirMissionsWidget.tsx) draws the table and drill; [airMissionsData.ts](airMissionsData.ts) validates replies and looks rows up; [useAirMissions.ts](useAirMissions.ts) requests the history and lists the settings' choices.

Shared framework conventions: [widgets](../README.md).
