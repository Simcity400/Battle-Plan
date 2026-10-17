# Factions

**What it shows:** Recorded factions and members on a lane timeline, with member-count bands and marked leader spans.

**Product(s) it reads:** [`country-groups`](../../../../crates/dashboard-base/src/products/country_groups.rs) supplies faction rosters, leaders, names, identity runs and unknown coverage.

**Settings:** All countries by default; manual/smart-group choices retain factions those native holders joined. Faction/member ordering, dissolved/former entries, colour legend and global/campaign swatch scope; Grow with playback; Campaign, Follow, Pin and Range.

**Notable behaviour:**

- Native member order zero supplies the leader. Missing identities make roster counts unknown; gaps in bars/bands never imply a join or departure. A continuing membership stays one lane across aliases, while a replacement under the same tag gets another lane.
- Manual roots select their native holders and aliases, excluding unrelated alias holders. Group choices preserve hour/end/ever counting dates.
- Literal faction names require the recording language; otherwise empty literals use recorded content templates. Missing reads or unsupported templates stay **Unnamed faction**, with one **Name unavailable** tooltip status. The legend shows faction names. Campaign display names override native names.
- Colours use shared campaign/global faction preferences, defaulting to the first recorded leader's map colour. The game's own override colour is not recorded. Member looks are dated and bounded by holder/coverage; headings stay flagless.
- Before preparation covers the branch, rosters and leaders after the prepared prefix come from each later block's keyframe: those bars take the faded dashed approximate edge, hours not yet sampled are banded as still being prepared, and one key line names both. Lanes are muted until the campaign's faction colours, which follow the complete history, load.
- Live reveals reached lanes and grows bars. The shared timeline owns zoom, keyboard, hover and Jump; labels use the shown hour, tooltips the hovered hour.

**Files:** [FactionsWidget.tsx](FactionsWidget.tsx) renders; [factionsData.ts](factionsData.ts) shapes native-holder lanes and bounds, with adjacent tests. [FactionColours](../../dashboard/FactionColours.tsx) supplies names/colours. `factions.css`, `definition.ts` and `index.ts` own styling, settings and registration.

Shared framework conventions: [widgets](../README.md).
