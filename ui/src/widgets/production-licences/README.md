# Production licences

## What it shows

A moment view (Follow or Pin) of the production licences standing at the date shown, as Trade flows' flow diagram: licensors on the left, licensees on the right, a ribbon per pair in the licensor's colour. Ribbon width is the number of licences, or with the Ribbon width setting the civilian factories lent (licences lending none are then left to the tooltips and the drill). Countries that do not fit fold into Other, which sums them exactly. A country's flag or name opens its licences table (licensor, licensee, designs, factories, since) in the drill.

## Products it reads

[`production-catalogue`](../../../../crates/dashboard-economy/src/products/production_catalogue.rs) in its `licences` view (family `production_catalogue`): each licensee's stored licence records from the daily production slot. Design names come from the variant catalogue at their naming reads.

## Notable behaviour

- A group whose membership was not recorded at the hour shown says so, and a group with no members then says it has none; neither reads as "none" for its countries.
- A licence grants designs; it does not prove anything was built. The record's opaque operand is not shown as a fee.
- A record's position is not a persistent identity, and a licence that stops being read only bounds its end; no end date is invented.
- Zero factories lent is a real licence; factories not recorded show an em dash.
