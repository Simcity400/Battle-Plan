# recorder-oracle-event-feed

Recorded event-feed consistency and explicit partial/unavailable interval limits. Owns [event_feed.rs](src/event_feed.rs), the `MapEvents` self-check in [map_events.rs](src/map_events.rs) (continuity, family coverage and save-hour joins) and their child tests. The [oracle command](../recorder-oracle/README.md) composes comparisons; this crate calls its specific dependent comparisons directly.

Table completeness, exact joins, differences and named limitations use [shared oracle support](../recorder-oracle-support/README.md). Content-dependent facts resolve through the recorded content stack. Synthetic fixtures are dev-only through the support crate's `test-support` feature.

Shared rules live in [AGENTS.md](../../AGENTS.md); the [recorder guide](../recorder/README.md) defines fresh-save/load and matched-native-observation checks.

Retained script-event comparisons reduce EventFeedHistory through the matched hour and compare pending IDs/dates/scopes, fired identity multisets and every current country instance's ordered delayed timers, duration components, originators and scopes. Random words follow save order; unknown saved fields and missing coverage fail. Pending originators and occurrence history are unsaved limitations, separate from compared membership.

Retained decision comparisons reduce the event stream through the save hour and compare taken order/multiplicity, all five timer/mission containers, signed durations, stored lifecycle, serialized target/ignore operands and ordered description random counters. They are independent of the daily census age and never waive a save difference.

Script-family refusal bits report genuine lost intervals independently of transport. Missing owner baselines or pass/writer certificates fail retained comparisons; specific unsaved trigger/presentation inputs are separate limitations.

Each retained domain reports its own reasons and never stops the others: an incomplete save-hour interval under `EventFeedHistory interval`, then `EventFeedHistory pending_events`, `fired_event_names`, `delayed_events` and `decisions`. Uncertified delayed and decision owners are grouped by reason with every tag listed; an owner whose saved operands cannot be read (an unknown saved field, say) is refused by tag alone. Every certified owner and registry is still compared and its differences reported, and any refusal keeps the comparison incomplete. Reasons for one group accumulate. An unreadable saved registry, delayed queue or decision status refuses only its domain or owner. Only an unnamed save country or a frame context that cannot be read fails `EventFeedHistory` as a whole.
