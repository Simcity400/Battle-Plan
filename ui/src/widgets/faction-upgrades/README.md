# Faction upgrades & rules

**What it shows:** How a faction's constitution evolved: lanes for its rules, shared programmes, faction minimum upgrades, members' upgrade selections and shared doctrines, under headings, each bar the selection held, named from content. The selection held at the shown hour is emphasised.

**Data it reads:** `faction-state` (`view: 'rules'`) for the chosen faction ([factionChoice.ts](../../dashboard/factionChoice.ts)): observed selections as runs between reads.

**Settings:** Faction; **Ended programmes** shown or hidden; Grow with playback.

**Notable behaviour:**

- A change is observed between two reads; no proposer, vote, tier or unlock date is shown, since none is recorded. Unrecorded hours stay blank, never an ending.
- A selection whose name the content does not define reads "Name unavailable", counted in diagnostics.
- A member's lane is named as its own holder was at its last recorded selection, never at the run without a selection that closes a lane once the holder goes.
