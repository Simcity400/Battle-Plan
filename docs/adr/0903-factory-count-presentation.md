# 0903. Derive factory counts from the same recorded country columns

The game's factory counts use the CountryFactories row and its military, naval and civilian column groups. Built is the stored total. Top bar subtracts damage and factories given away. Available also subtracts civilian consumer goods, contracts, projects, licences and trade imports from the civilian group. The channel sums only the requested groups. Missing required columns withhold the result instead of supplying an assumed count. Fixed5 totals truncate toward zero and must fit the displayed signed integer.

[Factory counts](../../crates/replay-derive/src/factories.rs)
