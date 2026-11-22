# Convoy health reports only recorded evidence

Regional efficiency uses a country prefix in its row key. Resource deals and convoy clients carry the country in a column instead. Index each run once by the row handle and the hours it names an importer, exporter or receiving country. Cut the history reads to those spans so a reused handle, changed importer, or branch gap does not contribute the prior owner's data. Resolve raw country reference and tag changes at the hours they apply. The history helpers use sorted, disjoint spans when clipping runs or finding the uncovered intervals.

The game multiplies the two fixed5 trade efficiency factors at a denominator of 100,000 with truncation toward zero. Compute the product in a widened type and leave it unavailable when a needed factor is unrecorded or the row is absent. Lend-lease convoy clients use category 4 and offer only the bad-route flag and assigned and required convoys. The dashboard has no numeric efficiency for those clients. The availability in the reply distinguishes a complete read with no matching row from an incomplete or missing read. Neither case invents an efficiency value.

Resolve strategic-region labels from recorded content, reporting an unnamed region by its reason. Resource tokens are keyed by native index and come from the timeline's last hour. An unspecified filter requests all recorded resources.

[Convoy product](../../crates/dashboard-economy/src/products/convoys.rs) · [Convoy coverage tests](../../crates/dashboard-economy/src/products/convoys/tests.rs)
