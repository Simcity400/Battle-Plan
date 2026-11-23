# Attribute hourly country totals from recorded ownership

Several writers derive country totals incrementally from the current recording tables. They rebuild their indexes after a resume or a discontinuity because a delta alone cannot establish a baseline. A change to country identities, attribution or the completeness of a required table also invalidates the affected indexes. These writers keep their durable state in their output and input tables, so a resumed run can match an uninterrupted one without serializing the indexes.

## Navy

`NavyTotals` and `NavyClasses` count each ship independently for the roster owner and the logical owner of its task force, in that order. A ship moving between forces changes both attributions. A missing force or unresolved owner invalidates every country's totals for that attribution. Class rows are withheld until all ships for that attribution resolve. A negative current hull strength withholds the hull sum for its country and class. Maximum hull strength uses the recorded model counter with a minimum of 100 fixed5 units. Incomplete model coverage or one missing counter withholds that sum. A live country with no ships still gets a totals row. The writer rebuilds after country slots, links or model completeness change. [Navy writer](../../crates/recorder-derive-naval/src/navy.rs)

## Nuclear stockpiles

`CountryNukeTotals` adds the whole bombs shown for each type instead of rounding the country's aggregate. The native sentinel preserves amounts outside the finite range. A missing stock cell withholds the country's total and ready count until it becomes recorded. A live country with a complete read and no stockpile holds a recorded zero. [Nuclear stockpile writer](../../crates/recorder-derive-air/src/nuke_totals.rs)

## Occupation

`OccupierTotals` assigns actively resistant states with a positive occupied-country reference to the state's controller. It uses the game's wrapping fixed5 sums so the query can divide each sum by the state count. An unresolved controller or absent state makes the sums and counts unknown for all occupiers. Held states are the union of occupied and garrisoned states. An incomplete garrison read falls back to the observed occupied states, and `held_complete` requires a known garrison for each occupied state. Effective occupation law stays in the query because its triggers must be evaluated there. [Occupation writer](../../crates/recorder-derive-world/src/occupation.rs)
