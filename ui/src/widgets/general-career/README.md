# General career

**What it shows:** One general or admiral's identity, skill, traits, commands and recorded battles. The standalone widget and Generals leaderboard drill share CareerView.

**Product(s) it reads:** [`unit-leaders`](../../../../crates/dashboard/src/products/unit_leaders.rs) supplies career/choice histories; [`unit-leader-portraits`](../../../../crates/dashboard/src/products/unit_leaders/portraits.rs) supplies separately cached artwork.

**Settings:** Choose a full leader reference from the country-filtered inventory; Grow with playback; Campaign, Follow or Pin. The leaderboard's inline/pop-out drill adds Back, Follow and Pin.

**Notable behaviour:**

- Full references retain careers across country transfers. Countries narrow choices rather than cutting the chosen history. Defined empty names stay empty; absent names show unavailable wording. Dated owner facts never borrow another holder's flag.
- Initial skill/trait holdings say **Held before**; gains after missing reads say **First observed**. Missing trait reads cannot manufacture a second holding date. Content defines ranks, stars/skill ceilings and artwork.
- Commands use recorded formation references and stored names; overlapping commands get separate subrows. Missing name reads do not end a command. Older captures expose their explicit fleet-name/idle-command limits.
- Fought means observed battles since the leader's first recorded hour. Known wins/losses use native links; undecided battles stay unresolved and zero decided battles have no win rate. Native participation/result dates become visible only after their own supplying reads.
- Gaps remain unknown. The shared timeline preserves zoom during growth and keeps label hours separate from drawing cutoffs. A picked drill day holds; Follow uses playback; Pin keeps the leader and held date or Follow.

**Files:** [GeneralCareerWidget.tsx](GeneralCareerWidget.tsx) resolves selection/time; [CareerView.tsx](CareerView.tsx) renders the shared [timeline](../../charts/timeline/README.md); `choicesData.ts` validates choices and `CareerView.test.ts` covers behavior. `definition.ts` and `index.ts` declare settings and registration. Shared leader data lives with [Generals](../generals/README.md).

Shared framework conventions: [widgets](../README.md).
