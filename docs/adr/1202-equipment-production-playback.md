# 1202. Hold production windows during playback

Equipment production is fetched in hour windows. The race prepares the visible window and the next one during playback. The line view prepares the windows in view. Both read the playhead from prepared data. The race repaints values, order and date while its set of rows is stable. Paused seeks still update the displayed hour when the rows are unchanged. Detail reads follow explicitly hovered or chosen hours, so holding a tooltip never asks per playing hour.

Top-N countries are ranked at the paused or sought hour and held while playing. Ranking at the branch end would select countries by their future output in an earlier race. Unknown values remain gaps and are ineligible for ranking. A continuous equal-value run can join across a window boundary. A new stretch does not join and unknown runs only join when their reasons agree. The displayed start of a cumulative total follows the latest observation that started it.

Units default to the most delivered type and are never summed across types. Industrial capacity is one currency and initially counts all types, so it supports shares and a total. Counts use whole units or fixed-five capacity; a small nonzero capacity is displayed as nonzero. The breakdown stacks either one type's models or all capacity by type, grouping the smaller types. A known total can leave a residual band after the known parts have been summed. A missing total leaves the stack blank. Output with an unrecorded type makes the affected type breakdown unknown even when its known models are complete.

Requests identify the country the drill asked for and names resolve at the selected read. Artwork uses the same dated stockpile artwork lookup as logistics, with a named placeholder for unavailable pictures. The settings escape content keys that cannot be stored verbatim.

[Production lines](../../ui/src/widgets/equipment-produced/equipmentProducedLinesProjection.ts) · [Production race](../../ui/src/widgets/equipment-produced/equipmentProducedRaceProjection.ts) · [Production breakdown](../../ui/src/widgets/equipment-produced/family.tsx)
