# Faction goals

**What it shows:** A faction's shared goals at the shown hour: each held goal with its progress and status, then the goals in the faction's completed and cancelled lists, grouped by term in the order the faction's goals first use them. Moment widget: Follow or Pin.

**Data it reads:** [`faction-details`](../../../../crates/dashboard-base/src/products/faction_details.rs) at the shown hour (the read Factions' details make), for the faction chosen through [factionChoice.ts](../../dashboard/factionChoice.ts): the widget's country's faction from the recorded rosters in `country-groups`, or one picked in **Faction**. Goal names, descriptions and term names resolve through the recorded content stack.

**Settings:** Faction; **Goals**: All, In progress or Completed (cancelled goals show under All, muted); the table's density, narrow-table behaviour and sort.

**Notable behaviour:**

- Progress is the game's held cached percentage; a goal without one reads "Not recorded", never 0%. A goal in the completed list is not assumed to be at 100%.
- The completed and cancelled lists carry no dates in the recording, so no completion or cancellation date is shown, and no contributors: the recording names none.
- An unresolved goal or term name reads "Goal name unavailable" / "Term name unavailable", counted in diagnostics.
