# Doctrine mastery

**What it shows:** One country's doctrine tracks at the hour shown, folder by folder in content order: each track's chosen subdoctrine and a vertical meter filling downward with its mastery, marked at every reward threshold the content authors, with the daily gain and banked mastery under it.

**Product(s) it reads:** [`doctrine-mastery`](../../../../crates/dashboard-doctrine-mastery/README.md), a keyed family prepared from the hourly doctrine tables, with folders, tracks, subdoctrines and rewards from the recorded content stack. One request per country holds the branch; playback looks the hour up locally.

**Settings:** Doctrine folders (also the chips above the tracks; all by default). Follow (default), Pin or the campaign's end.

**Notable behaviour:**

- The meter's scale is the subdoctrine's last authored reward threshold; marks sit at each threshold, however many and however spaced, reached as the recorded mastery says. A track without a positive threshold keeps an unscaled meter rather than an invented maximum.
- Daily gain is the game's effective gain; the units' own rate is kept apart in the tooltip. Banked mastery is per track.
- A track with no subdoctrine is muted, distinct from one at zero; unknown mastery says Not recorded. No unlock dates or time-to-unlock: the recording has neither.
- Folders and tracks a mod adds appear as the content defines them; tracks scroll sideways when they do not fit, wrap to two columns below 640 px, and below 420 px reward labels hide (marks stay, names on hover) and one folder shows.

**Files:** [DoctrineMasteryWidget.tsx](DoctrineMasteryWidget.tsx) draws folders and tracks; [doctrineMasteryData.ts](doctrineMasteryData.ts) validates the reply and builds each track's meter; [useDoctrineMastery.ts](useDoctrineMastery.ts) requests the history and lists folder choices. The vertical meter is the chart kit's [ProgressMeter](../../charts/meter/ProgressMeter.tsx).

Shared framework conventions: [widgets](../README.md).
