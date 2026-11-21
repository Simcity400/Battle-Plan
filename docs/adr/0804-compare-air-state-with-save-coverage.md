# Compare air observations within the save's coverage

## Daily air ledger

The native daily cursor identifies the destination physical slot. Hour timestamps alone cannot disambiguate writes on either side of a midnight advance, so the comparison includes the start-midnight drain and selects actual saved slots. The save keeps the current and previous physical records. Older drain entries fall outside the comparison. An allocated regional history without its daily window cannot certify a zero. Recorded event loss also leaves the interval unproven even if the table reads as complete. Nonzero recorded amounts with no saved queue differ. The missile credit channel can prove equality because it has no other indexed writer. Bombing, division or ship damage, logistics and enemy channels have additional writers. Country zero resolves to the reserved null identity. A target province may also return a null state. [Air ledger oracle](../../crates/recorder-oracle-air-ledger/src/air_ledger.rs)

## Missions and sorties

Compare saved base, mine and reassignment fields with the actual mission projections. Fields absent from text saves, including registration order and country vectors, require the same-hour census. A wing's progress and distance alone are gated by eligibility. Sparse demand flags can be known false, while unread demand remains incomplete. A console override still counts when its value is zero. History rows and refusals keep native positions, including null or reserved slots without saved tag identities. Certified full navy vectors supply the mine observations. The base witness covers every wing's pool or transfer base, even when its current mission is unrelated. [Mission and sortie oracle](../../crates/recorder-oracle-air-missions/src/air_missions.rs)

Air enemy vectors normally match the saved active war opponents as sets. A civil-war revolter whose declaration returns before rebuilding its enemies, or a country with a retired war relation, can have a stored vector that lags the save. For a same-hour `CountryWars` witness of that vector, compare the recorded vector in exact order and report the save's named limitation. An unread vector cannot excuse a difference.

## Regional air combat and names

For `AirRegionCombat`, apply the daily prune at the saved hour using the recorded content's timeout. The prune only affects countries that own a state. Compare every retained entry and both damage channels in native order, and treat an allocated holder that the prune cleared as an empty saved holder. The retired regional tables are outside this pack's comparison. The ace serializer always writes name and surname, while callsign presence is decided by the stored length. [Regional air oracle](../../crates/recorder-oracle-air/src/air.rs)
