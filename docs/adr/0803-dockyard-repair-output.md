# Reconcile dockyard repair output with base selections

The repair writer derives the total rate from the count of dockyards in use and one dockyard's fixed5 daily output. For each input, exactly one raw country belonging to the canonical slot must hold a row. Multiple matching aliases are ambiguous. A recorded count of zero gives a zero rate even without a recorded output. A missing count, or an unknown output for a positive count, withholds the total.

A base selects queued ships in queue order using the replay repair rule and stops at its allocation. Distinguish an absent row from a held row with unavailable operands, since a selection that requires one of the latter is unreadable. Publish the design split only when every base could be read, their allocations add to the known country count, and there are no excess selections. Each selected ship applies one dockyard's output to its design. The remainder is unattributed. Designs no longer selected lose their rows.

The writer indexes bases, queues and their ships and forces so an hour revisits only the selections its changes reach. Relation changes trigger selection when they affect ports and access. War casualties need no new selection, while a war starting or ending can change access. A fresh hour or resume rebuilds the indexes and outputs from the held tables, without a writer blob.

[Writer](../../crates/recorder-derive-naval/src/dockyard_repairs.rs) · [Input decoding](../../crates/recorder-derive-naval/src/dockyard_repairs/inputs.rs) · [Tests](../../crates/recorder-derive-naval/src/dockyard_repairs/tests.rs)
