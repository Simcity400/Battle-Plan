# 0403. Invalidate held reads at the writer that owns the change

Several recording tables keep authenticated data between captures. The observer names the affected object and leaves the next reader to copy it. The observer does not call into the game to refresh the value. Construction, replacement, load and teardown are separate lifetime boundaries. A witness from an earlier world must never authorize carry in a new one.

## Division equipment and orders

The division panel tracks membership in demanded equipment pools. Addition, setting, scaling, clearing and assignment cover membership changes. Subtraction goes through addition. The design family forwards variant rebuilds and assignments, and construction or loading revokes all certificates.

Ordered membership observes inline assignment, deletion-only removal and root and parent repairs as well as ordinary appends. The append's first operand is the order's embedded member vector. For orders-group census inputs, observe the leader setter, the HQ-provider selection and clear, and changes to training. Parenting can copy the parent group's training to its child. The daily update can clear a group that has reached its training limit. Identity is set at construction and registration, before divisions can refer to the group. Destruction is forwarded by the template family. Load and repair use the new epoch.

## Provinces

Combat vector changes go through unique append and swap-removal. Append covers land and naval creation, joining a naval combat, and rebuilding links after a load. All removal paths finalize the combat before deleting it. Province loading and teardown also write the vector, so resets and world replacement revoke the witness.

Province occupant order is preserved. Occupants append on location changes and are removed by a stable compacting erase, including deletion, transfers, exile and withdrawal. Moving to the same province still removes and re-appends. A fleet merge can therefore move the survivor to the end without changing counts. No runtime writer sorts the list or inserts into the middle. The location setter is bracketed by erase and append, and occupant handles and classes are established on construction or load. An untouched list can reuse only the same handles, classes and backlinks in the same epoch.

## Naval ledgers

Registration appends to the region's ledger and to the navy's registration set when missing. Withdrawal erases from all ledgers and from the set. The world-input family forwards the reset of both structures. The naval map family owns empty construction, regional growth and persistent loading, and loading changes the epoch. Convoy-escort presence is rebuilt in the mission update and remains an hourly read.

## Faction and diplomacy state

Faction companion capture follows sparse changes to rules, programme membership and manpower. An authenticated faction census bounds identity creation and deletion. A daily programme award does not edit the programme list, so only list writers invalidate it.

Faction goal structure is tracked through insertion, removal, activation, selection, and load or rebuild. Activation includes the term date, and transition completion includes the resulting status and dates. Compare scalar progress refreshes to the held value. Mark actual pending-status stores and observe the cancellation-history append only at its owning call. The two history append paths are distinct. Construction, destruction and world replacement bound receiver validity.

Relation opinion recomputation does an unconditional store even on daily workers. The observer must support those workers and mark only changes. Reset can also occur during civil war, annexation, state transfer or dynamic-country reuse. Do not infer that affected statuses stay alive after their diplomacy owner is freed. Replacing a relation can swap-remove the old entry and append to subrelations. The serializable vector belongs to the first endpoint's status, though apply and clear can name both endpoints. War replacement is forwarded by the country-input family.

## Advisors

The advisor family observes construction, destruction, idea changes and parsing. Factories initialize beyond construction, and map publication is an independent boundary. Shared insertion code also serves template maps, so key the event by the exact map address. Merge can insert inline. Range erase, recursive clear and character destruction close lifetimes, including those of empty maps. A fresh-read diagnostic cannot establish writer coverage.

[Division equipment](../../crates/engine-mutations/src/division_panel_mutations.rs) · [Order membership](../../crates/engine-mutations/src/order_membership_mutations.rs) · [Orders groups](../../crates/engine-mutations/src/orders_group_mutations.rs) · [Province combats](../../crates/engine-mutations/src/province_combat_mutations.rs) · [Province occupants](../../crates/engine-mutations/src/province_unit_mutations.rs) · [Naval ledgers](../../crates/engine-mutations/src/navy_ledger_mutations.rs) · [Faction companions](../../crates/engine-mutations/src/faction_companion_mutations.rs) · [Faction goals](../../crates/engine-mutations/src/faction_goal_mutations.rs) · [Opinions](../../crates/engine-mutations/src/opinion_mutations.rs) · [Advisors](../../crates/engine-mutations/src/advisor_mutations.rs)
