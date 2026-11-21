# Autonomy lanes use recorded levels and the holder at the shown hour

Each subject has one lane covering its subjection runs under any overlord. Selection can match an overlord in those runs or the subjects themselves. The level order and names come from the recording's content. Colours follow that order so renamed or modded levels keep their relative colours. The label distinguishes a subject with an unrecorded level from a tag that was not a subject then. Touching periods can share a progress span without filling unrecorded gaps, and change markers come from level changes inside a run.

The timeline takes level bands from the subjects reply and progress from the declared autonomy series. The normalized fixed-five scale reaches its upward threshold at 100,000. Values beyond a threshold are painted at its edge. Each lane follows its current progress between renders, with threshold details read only for the hovered hour. Tile arrivals repaint without replacing the lane structure.

The identity lookup is bound to the tag's holder at the hour shown. A lane shows the tag and a missing-flag explanation when the country did not exist at that hour, so a different holder's name or flag cannot appear. Order and labels need all the names, while flags can be read only for lanes in view.

[Autonomy data](../../ui/src/widgets/autonomy/autonomyData.ts) · [Autonomy widget](../../ui/src/widgets/autonomy/AutonomyWidget.tsx)
