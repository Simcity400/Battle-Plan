# 0701. Preserve identity and absence when reading native definitions

## Agency upgrades

The branch registry owns authored agency upgrades. The loader reuses branches across files and can rebuild a same-name definition inside a branch. Nonzero branch validity bytes remain valid, and the member array contains no sentinel. The database's global definition vector reserves its first slot for null and retains the first pointer for a repeated token. The branch vector also reserves its first slot for null, which is included in its signed count. Validate the singleton and branch layouts before reading their arrays. Resolve authored identities using the live lexical dictionary and retain branch-owned definitions even if the global vector omits them. The second-branch duplicate-token test covers this case.

The native acquired array stores one definition pointer and signed level per occurrence, independently of the agency's created flag. The definition's level cap does not clamp the observed acquisition. Keep the observed order and raw entries. Their ordinals describe one read and carry no persistent identity. Acquire branch data only from an authenticated definition layout. Zero in the raw branch field means the pointer was unavailable, while portable identity has its own refusal status. Capacity omissions count separately. A changed identity witness invalidates portable identities for the read and leaves the raw entries intact.

Demand checks read the country and agency identities and validated array headers without decoding any upgrade definitions. Rebuild the caller's identity scratch in each combined agency walk. Prepare or grow the scratch off the sampling thread and never retain the catalogue's native pointers across hours.

## Allocation and autonomy records

Garrison allocation slots have no established enclosing native type. Read every slot separately using the signed equipment lookup key and signed fixed-point amount. Preserve order and signed values rather than combining, filtering or clamping allocations. Capture addresses only as callback context. The owner authenticates the bounded array, and any omitted slots remain truncation. An unclassified field is outside this table's contract.

The subject and current-autonomy readers obtain the name string from definitions reached through their own relations. Treat those definitions as reference inputs without assigning an unverified native type.

[Agency catalogue](../../crates/engine-native-intelligence/src/agency_upgrade_levels.rs) · [Branch registry](../../crates/engine-native-intelligence/src/agency_upgrade_branch_database.rs) · [Garrison allocation](../../crates/engine-native-parts/src/allocation_equipment_row.rs) · [Autonomy definitions](../../crates/engine-native-parts/src/autonomy_state_definition.rs)
