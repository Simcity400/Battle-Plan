# Admit legacy directed-war packets once per owner and hour

Recordings through format 101 can store exact directed-war cells in `CountryCounterInputs`. Current recordings derive them from WarRelations. Historical admission validates the recorded arrays and associates the packet with the hour and receiver resolved from CountryLinks. Query through the borrowed context so callers cannot change its identity or domain between target lookups. Accept a differing hour only with the historical stamp.

The native test uses the resolved slot, including zero. A null slot-zero war pointer or a retired relation is false. A complete writer-witnessed active domain allows a false cell to be omitted. A partial read without a cell leaves the answer unknown. Receiver zero can reuse the recorded reverse-visibility war value. Bounds checks and binary searches suffice after admission, including for repeated queries.

[Historical directed-war packet](../../crates/replay-rows-presentation/src/directed_war.rs)
