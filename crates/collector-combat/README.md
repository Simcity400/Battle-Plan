# collector-combat

Owned land-combat details, participants, casualties, modifier operands, reinforcement inputs and ReserveModifierTrees (each reserve's country and selected commander unit modifier reinforcement-rate tree, printed form, commander name only when its rate is nonzero; a source whose read fails is that row's refusal; country and commander unit-modifier trees carry while the naval tree writers' certificates hold, one carried tree a pass compared with a fresh read), with their native acquisition and replay conversion. The host supplies the source and hour and owns dispatch, loans and table assembly.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

The ordinary combat worker copies reserve-speed operands in native read order; FIFO decode composes their shares and diagnostics without a Memory capability. Unfinished owned settlement forgets the held reserve-tree baseline before reuse; tree verification and native-demand decisions remain parked.

Run `cargo test -p collector-combat` and `cargo clippy -p collector-combat --all-targets -- -D warnings` for its synthetic checks.
