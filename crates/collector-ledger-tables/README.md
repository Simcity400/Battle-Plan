# collector-ledger-tables

Event store tables: the hourly packs that drain the event feed history and the air loss, building bombing and nuclear strike event ledgers in collector-observers into owned rows and convert them. The host owns dispatch and table assembly.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-ledger-tables` and `cargo clippy -p collector-ledger-tables --all-targets -- -D warnings` for its synthetic checks.
