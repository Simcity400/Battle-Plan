# 0003. Publish variable names only from verified installation documentation

The live variable table reads the installation's dynamic variable documentation. It preserves each identifier exactly as written and sorts names by their bytes before validation. The expected digest for each supported scope is tied to the names in that scope. An absent scope, duplicate name, or changed required list withholds the table rather than publishing an incomplete one.

The documentation names the industrial organisation scope `military_industrial_organization` and the project scope `special_project`. These are content names. The recording stores the verified lists for playback without an installation lookup.

[Variable tables](../../crates/engine-executable/src/executable_tables/variables.rs) · [Recorded executable tables](../../crates/engine-executable/src/executable_tables/recorded.rs)
