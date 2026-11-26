# Resolve dashboard trigger scopes from recorded inputs in native order

The same trigger evaluator serves several dashboard descriptions and checks. Its scope transitions must match the game's evaluation, including the order of reads and handling of unavailable inputs, so a true or false reply carries the same meaning on replay.

## Loading and branching

- An unrecognised key can add no child. An otherwise empty conjunction keeps its normal identity, and a later `else` can still attach across the ignored key. If it is unknown whether a key names a recorded character template or country, evaluation withholds the affected result.
- The first `limit` that loads a child owns the limit. An earlier limit that loads nothing leaves the slot open. Each conditional checks its own limit, including `else`. An inner alternative joins the last inner `if`, or the enclosing chain if none exists. A top-level alternative following an attached `else` adds nothing.
- Ordinary evaluation short-circuits by reachability. A colour callback also records the identity of each missing input it actually reached. A known final Boolean result does not hide those inputs. Start a fresh refusal set for each callback and distinguish entities even if their refusal text matches.

## Country aliases and scope carriers

- Original-tag selection reads recorded original-tag buckets and the physical country array as distinct facts. Numeric original-tag comparisons use raw references through the country links. Text labels and live party or appearance fields must not replace a recorded identity key.
- An unscored alias chooses the first passing candidate. Read the original-tag country first, then that bucket's dynamic countries, then any declared targets in canonical slot order. A whole-country scan uses the physical array order and needs its complete roster. A scored alias chooses the highest positive score and leaves an earlier candidate in place on a tie. A fallback runs only when nothing passes. Repeated original-tag assignments and recursive alias cycles are refused by name. An unreached alias input creates no refusal.
- Each candidate owns a fresh scope, with its caller as `PREV`. Setting a country clears unrelated carried objects and retains links. `THIS`, `FROM`, `ROOT` and `PREV` copy scope links, so a nested link reads the linked scope's own links. Only the combat scope describes those links to itself. Combat resets the ordinary country carrier. The `tag` check still admits combat and reads its country through its explicit getter.
- Character targets are identified by recorded content templates under the lexer's fold, then resolved in `THIS`'s country. A template without a live character leaves an empty carrier and fails a nonzero scope mask. An absent template key adds no child. The null state holds no object. Mask-zero checks can still run there, and the variable evaluator uses the global store.
- Recorded reserved-country data is independent of the positive country census. Its own getter can prove a null faction or an empty roster even when an unrelated positive table is unavailable. A literal reserved owner in a member or theatre row is distinct from an unresolved positive country. A missing positive link can hide a matching row and must remain unknown.

Unit-leader visibility first resolves the recorded template identity and role from its token, file and line; a template made by `generate_character` instead takes its role's recorded visible state and source. An absent `visible` conjunction is true. A present one is then evaluated in the owner's scope. Current ideology does not change a recorded role-map key for a party leader. The complete role census distinguishes a present role with an empty trait array from a role the recording lacks.

Keep the parser's handling of a three-part alias name on the failure path. It reports that form and retains the event name from the start through the length of the second part, with no variable. The generic `global_event_target` token remains a parsed token even where that context has no scope case for it.

[Trigger parity tests](../../crates/dashboard-text/src/triggers/tests.rs) · [World inputs](../../crates/dashboard-text/src/triggers/world.rs) · [Reserved-country tests](../../crates/dashboard-text/src/triggers/world/zero_countries_tests.rs)
