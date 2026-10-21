# collector-advisors

Hired advisors capture: appointed and available advisors, advisor tokens, the country and retired character records, and the held walks that carry a quiet hour, using the advisor pool, character and roster observers in collector-observers. The host owns dispatch, loans and table assembly.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-advisors` and `cargo clippy -p collector-advisors --all-targets -- -D warnings` for its synthetic checks.

Spool projection compares complete appointed/active/retired rows before replay
conversion. Equal advisor token sets retain the encoder's shared complete
snapshot after the existing literal ordering and duplicate checks, including on
keyframes. Native acquisition, certificates and AvailableAdvisors slicing are
unchanged; refused arenas and partial sets cannot establish equality.
