# 1202. Keep battle state bounded by the recording's dated reads

Merge battle windows in recording order regardless of network arrival. Prefetched windows never supply facts to earlier hours. A result and verdict appear only after the read that settles them, even if the result is dated earlier. When a result omits participants already known from a census, keep the participants from their supplying read. Absence from an unended census is not a defeat. A battle already in progress at the start of its recorded stretch only has a lower bound on length. For a known naval ending, use the native linked length without extending it to the result's timestamp.

The land battle log asks for a bounded page of battles at the displayed hour. Ongoing battles lead the dated list. Numeric sorts use the two sides' recorded totals at that hour. The open detail keeps the selected read and requests the course and terrain for that battle.

Naval fleet details read at the start of each held run, and report pending or refused status until a detail is supplied. Merge each compact loss delta against its previous read. Raw equipment stays exact even where the display number would exceed safe integer precision. The course chart breaks on unread spans and scans for bounds without passing an unbounded read list as call arguments. Use exact equipment in the tooltip even when the visible label shows whole amounts.

[Land battle log](../../crates/dashboard-land/src/products/land_battles/tail.rs) · [Battle course](../../ui/src/widgets/land-battles/landBattleCourse.ts) · [Naval battle windows](../../ui/src/widgets/naval-battles/navalBattlesData.ts)
