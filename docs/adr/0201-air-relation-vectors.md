# 0201. Reuse flying-map relation vectors with watched owners

The flying-aircraft map uses three vectors for the viewing country. Capture splits the native country census into eight parts by slot modulo part count. Workers stage country-address, diplomacy-status and receiver-sequence evidence alongside copied vectors. Each country's two watch sequences are acquired before reading its data. A quiet epoch can skip guarded reads only if the cached census still identifies every slot and each present receiver is watched. An unobserved receiver must be read again even after a quiet hour. Diplomatic reset and wartime ally rebuild also inform the country-input observer, and reset informs the war observer because it clears that country's wars.

The scheduler offers capacity for two watched receivers per country, retires replaced tables off the game thread, and advances invalidation at the game-thread boundary before capture. A change of country object or diplomacy status cannot carry old vectors. Large destination copies grow within the worker after validating native header lengths against the live country links. Allocation refusal fails the part and its certificates instead of applying partially staged data.

The decode thread consumes frames in order, applies staged copies and removals to held slot rows, then expands the selected slice in slot order. Generations distinguish changed row contents and changed country identities across slots. A dropped frame withdraws the certificates it advanced. An undispatched part or failed staging withholds the affected frame rather than counting an empty array as known. The host owns staging loans, fingerprint lifetime and table assembly.

The relation observer's `Hooks::drop` releases the watch registry after captures have joined and either the hooks were unpublished or all installed hooks were restored and drained. Both callbacks and guarded copies are then quiescent.

[Air relation capture](../../crates/collector-air-relations/src/air_relations.rs) · [Relation observer](../../crates/collector-air-relations/src/relation_changes.rs)
