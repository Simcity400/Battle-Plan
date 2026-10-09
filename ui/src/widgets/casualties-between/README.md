# Casualties between countries

**What it shows:** Directed known casualty flows between countries on a chord, with complete signed pair details and histories.

**Product(s) it reads:** [`casualty-pairs`](../../../../crates/dashboard/src/products/war_casualties/pairs.rs) shares the [`war_casualties.rs`](../../../../crates/dashboard/src/products/war_casualties.rs) branch scan with [Casualties](../casualties/README.md).

**Settings:** All countries by default; chosen countries' casualties with anyone or only between them; countries around the chord, Chord/Recorded casualty pairs and inline/pop-out drill; Campaign, Follow, Pin and Range.

**Notable behaviour:**

- The chord folds excess countries into **Other**; every pair remains reachable in details even without a ribbon. BigInt preserves signed counters; only positive amounts draw geometry. Zero is an exact row, while an absent relation supplies no flow or invented zero.
- Manual roots select native holders and their dated aliases, excluding unrelated alias holders; smart groups use their counted membership dates. A pin retains campaign/branch-scoped pair holders, never replacement countries.
- Recording-block windows preserve carried read dates and prefetch in Follow. Missing/unloaded windows clear the chord; gaps never bridge. Unknown-attacker losses have no enemy and are excluded. These counters cannot separate military branches or causes.
- Incomplete values say **Recorded subtotal**, not a lower bound; unseen wars/final ticks remain unmeasured. Picking a history day holds; Follow uses the actual clock even when the chord is held. Outside-branch picks remain unavailable without clamping, and keep the picker usable.

**Files:** [CasualtyPairsWidget.tsx](CasualtyPairsWidget.tsx) renders and loads windows; `casualtyPairsData.ts` validates; `casualtyChord.ts` folds/projects flows; `casualtyPairDrill.ts` shapes histories and pins. Adjacent tests cover those rules. `casualtyPairs.css`, `definition.ts` and `index.ts` own styling, settings and registration.

Shared framework conventions: [widgets](../README.md).
