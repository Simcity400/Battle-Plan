# 1106. Preserve independent land combat condition operands

Land combat derives condition and survival scores from the same hour’s recorded tables. `Divisions` supplies current strength and organisation. `DivisionPanelStats` supplies maxima. A missing maximum leaves a division eligible for organisation scoring and withholds the early damage calculation only when that calculation needs the operand. Survival uses the full ordered census, including reserves. Missing condition or member-detail operands retain the table and division that could not supply them.

The early estimate evaluates firing committed divisions and verified game defines. Avoid chances are used as named, and fire from attackers tests defence while fire from defenders tests breakthrough. Current strength is expressed in tenths of maximum strength with a one-tenth minimum. A zero maximum or an out-of-range current strength gives full strength. A negative ratio clamps before applying the minimum. The later estimate uses the recorded loss ring and needs no strength maximum.

The estimates and progress use wrapping 64-bit sums and products followed by signed division with a positive denominator. Extreme reserved values produce the game’s positive sentinel. Nonpositive late loss averages take the native minimum denominator. When the modifier table is available, missing entries and feature-ineligible entries contribute zero. Contextual and country lookups both honor the metadata’s unit-interval clamp.

[Condition and survival inputs](../../crates/viewer-engagements/src/land_combat_math.rs) · [Operand and overflow cases](../../crates/viewer-engagements/src/land_combat_math/tests.rs)
