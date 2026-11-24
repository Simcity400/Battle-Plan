# 1404. Land combat openings and division exits

## Context

The land battle course came from hourly reads and the final result. A battle that started and ended between two reads had no roster, the closing roster was the last hourly read rather than the game's, and no division's losses in a battle were known: the hourly casualties counter mixes every battle a division fought and every other cause.

The game builds a land combat's whole opening roster in its combat-start routine: it creates the combat and both sides, adds the attacking units, the defender and the other units already in the province, and returns the combat. Units that join later go through the combat's own join, which the hourly reads see. The game realises a division's land-combat losses only when that division leaves a combat: its combat-exit routine turns the damage pending against the division into an applied loss and adds the men lost to the division's lifetime casualties counter. This runs for a division that withdraws, retreats or is routed mid-battle and, at the battle's end, for every division still in it. Damage pending in the combat is not applied before that.

## Decision

- **Two hooks**, installed with the land-event family: the combat-start routine, read after it returns, and the division combat-exit routine, read before and after it runs. Both are their own group: if either fails to locate or construct, both are withheld by name (collector status `land_combat_edges_withheld`), `LandCombatEdges` is unavailable with that reason, and the rest of the family stays installed.
- **Opening.** For a land or border-war combat the game registered, one row per member: side, division, manpower and lifetime casualties at the opening. Naval combats and combats the game never registered (interface previews) have no edge.
- **Exit.** One row per division exit: the side it was on, its casualties before and after the loss flush, and its manpower after. After minus before is exactly that division's realised losses in that combat, including a division that left early. The exits at a battle's end hour are its closing roster.
- **Never guessed.** Anything unreadable is withheld by name in the row (`withheld`): the combat, the division, the side or the hour. A division the exit no longer identifies after the flush keeps its before-count and has no after-count. A record never fails the land-event drain, because one decode failure would withhold the hour's battle results and destroyed divisions as well.
- **Storage.** `LandCombatEdges` is a new event table keyed by combat and journal position, so a battle's edges are one key-range read. Recordings before format 149 open with the table missing and the course says its openings and exits were not recorded.
- **Not covered.** Border wars start outside the combat-start routine, so their openings are not recorded; their exits are. A battle already running when a stretch of recording began has no opening.
