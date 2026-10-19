# collector-hooks

Scalar-store Tap spans admit authenticated stack-independent register clamp/loop operands and register immediates when a short store needs a longer patch. Stack/RIP operands, calls and branches remain refused. Synthetic execution verifies native flags, clamped stores and loop increments across the bridge.

Windows x64 patch publication, thread suspension and synchronization, Tap bridges and publication measurements. These ordinary Rust library modules are linked into the collector DLL. They preserve native registers, flags, unwind registration and continuation, and retain executable storage whenever restoration or quiescence cannot be proved.

Tap spans admit stack-independent indexed MOV/ADD and byte/dword immediate
stores. Stack-relative and RIP-relative operands, base-less indexed addresses,
branches, calls and unsupported opcode forms remain refused. Site owners still
authenticate the full sentinel, instruction span and native unwind scope.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-hooks` and `cargo clippy -p collector-hooks --all-targets -- -D warnings` for its synthetic checks.

`Hook::set_family_enabled_shared_named` preserves the shared publication path and its original failure cause. Site-specific checks retain a flag without allocation while threads are suspended; diagnostics format the family and failing site names after resume. Whole-publication failures name all participating sites.
Tap instruction relocation accepts non-stack, based SIB memory operands. RSP,
RIP-relative, unbased SIB, branch and call operands remain refused. Synthetic
store tests exercise indexed destinations, operand capture and flag preservation.

A Tap caller authenticates its full sentinel before construction. Publication
and restoration verify only the complete instruction span the Tap owns, so an
adjacent Tap can change its own instructions within that authentication window.
Both spans remain included in shared publication and active-thread exclusion.
The counter-publication Tap preserves its displaced memory increment and compensates a following stack-local reload for the bridge CALL return slot. It retains the existing register, flag and unwind guarantees.
The counter-publication Tap preserves its displaced memory increment and compensates a following stack-local reload for the bridge CALL return slot. It retains the existing register, flag and unwind guarantees.
Tap admits explicit stack-independent integer data instructions, including byte/immediate stores and register call preparations, and MOVUPS. It refuses SIB, RIP/stack operands, calls, branches and unsupported opcode extensions. Callers authenticate whole instruction spans and exclude interior branch targets and native return PCs.

Adjacent taps can publish only their owned instruction spans after callers authenticate their full construction sentinels. Untouched sentinel tails may then contain a neighbouring tap's patch; owned spans must remain disjoint.
`Hook::matches_published_snapshot` supports post-publication installation diagnostics. It compares the currently owned snapshot after threads have resumed; it grants no publication certificate and changes no ownership.
Tap instruction admission includes stack-independent event-option LEA, register MOV, mission-register zeroing, the fired-set erasure count, reached lifecycle/duration stores and the taken-decision removal operands. Targeted cooldown and cancellation-duration stores with their terminal jumps use an owned native continuation, preserving the original stack and unwind destination. Hardware shadow-stack enforcement refuses the redirected native continuation before publication. Runtime fixtures check execution, operands, flags and unwind; unrelated stack-relative, instruction-relative and branch operands remain refused.

Duration-adjustment taps admit the exact signed add-operand moves before the selected scalar helper. Their native fixture exercises the original register operands, flags and subsequent scalar add for both mission and removal-timer layouts.

Sparse decision taps admit exact register TEST/conditional-default moves, selected ignore stores and candidate-clear stores. Native fixtures cover both selection default branches, preserved flags and the final scalar result. Static family/site names identify publication byte mismatches.

Only explicitly opted publication families retain the first busy hook or tap from the existing suspended-thread scan. Ordinary families perform no diagnostic marker resets or updates, and no family rescans suspended threads for diagnostics. A not-ready publication remains retryable; a later successful tracked publication clears its prior busy marker. The marker is read only for attach deadline diagnostics.

`hook::installation::capture` observes real Hook/Tap construction attempts and typed sentinel, construction, publication and rollback failures on the attach thread, including errors an owning API handles internally. Sentinel predicates perform the same single guarded read as their caller. Observations are thread-local, scoped and absent from native callbacks. Native publication inspects retained rollback state only on error, after thread resume; serial and shared attach reports retain typed rollback failure even if an owner's later disable succeeds. Rollback failures retain their first cause.

Shared attach publication retains a typed failure for each group after native resume. Every stored-outcome route, including `attach_outcome` before reference rebuilding, reports that retained classification. The runtime visits all rollback failures before ordered family reporting; an earlier nonfatal error cannot discard a later group's fatal rollback or its family attribution.
