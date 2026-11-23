# 0704. Carry controller areas only with a complete change witness

The selected land route search reads controller area contacts in the order the game stores them. Use the smallest member ID as the label for both controller and owner areas. Controller area membership is sorted by ID. Its representative is the land province chosen by insertion, selected again when it leaves. It is a separate operand from the label and is accepted only if the province authenticates as a member of that area. Owner areas require pointer equality and null checks only. Native pointers stay inside the capture.

The province walk supplies both pointers from the same complete province census. A quiet hour may publish held areas without comparing pointers only with matching observation epochs, complete held rows and no marked province or forced refresh. A gap, failed job, incomplete walk or refused observation loses this reuse. A new epoch reads everything.

A marked hour compares the marked provinces, their neighbours and the held members of their touched controller areas. A split can move only members of the old area, and a join can move only members of areas already reached through the changed province or its neighbours. A later change can therefore reach only a union of the held areas already identified. This bound is unproven for a missing controller, a controlled province with a held null area, or an unreadable province or neighbour. Compare all province pointers in those cases. Re-read the areas the changed provinces are now in, the areas they left, and their neighbours' areas.

Read contacts as an ordered list. A node marked for deferred removal may already lack a live payload, so withhold its contacts rather than following it. A contact outside the captured areas has no label. Node or contact faults are local where they can be named, and a contact refusal leaves an authenticated representative available. A build that fails before publication leaves no current result.

The synthetic fixtures exercise quiet carry with a poisoned area, a new epoch, bounded and unbounded movement, deferred removal, mismatched membership, and contact targets without labels.

[Area walk and store](../../crates/engine-native/src/cprovince/controller_areas.rs) · [Area tests](../../crates/engine-native/src/cprovince/controller_areas/tests.rs)
