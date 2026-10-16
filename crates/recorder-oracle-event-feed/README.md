# recorder-oracle-event-feed

Recorded event-feed consistency and explicit partial/unavailable interval limits. Owns [event_feed.rs](src/event_feed.rs), the `MapEvents` self-check in [map_events.rs](src/map_events.rs) (continuity, family coverage and save-hour joins) and their child tests. The [oracle command](../recorder-oracle/README.md) composes comparisons; this crate calls its specific dependent comparisons directly.

Table completeness, exact joins, differences and named limitations use [shared oracle support](../recorder-oracle-support/README.md). Content-dependent facts resolve through the recorded content stack. Synthetic fixtures are dev-only through the support crate's `test-support` feature.

Shared rules live in [AGENTS.md](../../AGENTS.md); the [recorder guide](../recorder/README.md) defines fresh-save/load and matched-native-observation checks.
