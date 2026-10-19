# Factions

**What it shows:** Recorded factions and members on a lane timeline, with member-count bands and marked leader spans.

**Product(s) it reads:** `faction-details` supplies the selected faction's research/manpower access, rules, shared programmes, manpower pool, manifest, active/pending goals and cached progress, completed/cancelled histories, extra slots and dates at the shown hour. [`country-groups`](../../../../crates/dashboard-base/src/products/country_groups.rs) supplies faction rosters, leaders, names, identity runs and unknown coverage.

**Settings:** All countries by default; manual/smart-group choices retain factions those native holders joined. Faction/member ordering, dissolved/former entries, colour legend and global/campaign swatch scope; Grow with playback; Campaign, Follow, Pin and Range.

**Notable behaviour:**

- Native member order zero supplies the leader. Missing identities make roster counts unknown; gaps in bars/bands never imply a join or departure. A continuing membership stays one lane across aliases, while a replacement under the same tag gets another lane.
- Manual roots select their native holders and aliases, excluding unrelated alias holders. Group choices preserve hour/end/ever counting dates.
- Literal faction names require the recording language; otherwise empty literals use recorded content templates. Missing reads or unsupported templates stay **Unnamed faction**, with one **Name unavailable** tooltip status. The legend shows faction names. Campaign display names override native names.
- Colours use shared campaign/global faction preferences, defaulting to the first recorded leader's map colour. The game's own override colour is not recorded. Member looks are dated and bounded by holder/coverage; headings stay flagless.
- Before preparation covers the branch, rosters and leaders after the prepared prefix come from each later block's keyframe: those bars take the faded dashed approximate edge, hours not yet sampled are banded as still being prepared, and one key line names both. Lanes are muted until the campaign's faction colours, which follow the complete history, load.
- Faction details opens an hour-specific detail panel inside this widget. It selects one current faction, resolves goal definitions through recorded content and windows long goal/history, rule, shared-programme and manpower lists. Programme owner, active state and project names come from one certified supplying read of the existing daily programme facts; the section shows that read date independently of the current reference list. Each section distinguishes empty lists from unavailable reads. Individually unavailable operands retain named, counted reasons beside their sections. The detail read is independent of timeline preparation; missing reads or names remain unavailable.
- Live reveals reached lanes and grows bars. The shared timeline owns zoom, keyboard, hover and Jump; labels use the shown hour, tooltips the hovered hour.

**Files:** [FactionsWidget.tsx](FactionsWidget.tsx) renders; [factionsData.ts](factionsData.ts) shapes native-holder lanes and bounds, with adjacent tests. [FactionColours](../../dashboard/FactionColours.tsx) supplies names/colours. `factions.css`, `definition.ts` and `index.ts` own styling, settings and registration.

Shared framework conventions: [widgets](../README.md).

The detail panel also shows member influence/projection terms and shares, initiative and contribution breakdowns, selected upgrades, faction/system resource pools, research/doctrine sharing and advisor/spymaster modifier sources. Projection, shares, rank, initiative and influence totals use recorded refresh operands; programme support levels identify their daily supplying hour. Additional sections show held active goal-section sources, command permissions and contribution explanations. Individually missing operands retain their named reasons and counts.
