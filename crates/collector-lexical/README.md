# collector-lexical

Lexical capture: the game's interned token dictionary, its owned raw copy, growth hints and conversion into LexicalTokens. The host owns dispatch, loans, the spool conversion cache it lends and table assembly.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-lexical` and `cargo clippy -p collector-lexical --all-targets -- -D warnings` for its synthetic checks.
