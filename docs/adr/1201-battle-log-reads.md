# The battle log shows recorded state changes at the playback hour

Land and naval battle logs use a bounded page at the displayed hour. The branch index places ongoing battles first in date order and ranks numeric columns by the two sides’ totals at that hour. Its next change covers new observations, results, gaps and changes in the rows on screen.

Naval battle tooltips load the recorded fleets and named ship losses for the inspected row. A followed row uses the start of its current reading. A pinned live battle uses the displayed hour.

[Land battles](../../crates/dashboard-land/src/products/land_battles/tail.rs) · [Naval battles](../../crates/dashboard-naval/src/products/naval/battles/tail.rs)
