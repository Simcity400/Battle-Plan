# Convoy health

## What it shows

One country's convoys in Air superiority timeline's heat strips, a column per day: by default its delivery efficiency in each strategic sea region (By region), or with the View setting (and its own catalogue card) the trade efficiency each of its resource import deals gets, a row per deal (exporter · resource), plus a row per lend-lease shipment it receives. Each cell fades from theme grey (0%) to full colour (100%) by the day's read. The most active rows (most days read up to the playhead, the day under way at its reached read) come first, up to Number of rows (10 by default). Clicking a row (the heat kit's row pick) opens its drill: the row's facts at the picked day and its history over time (a region's efficiency; a deal's efficiency, both factors and its convoys assigned of required; a lend-lease shipment's convoys and bad-route flag), with Follow.

## Products it reads

[`convoy-health`](../../../../crates/dashboard-economy/src/products/convoy_health.rs), prepared on its own (family `convoys`) from `RegionalConvoyEfficiency`, `ResourceExchanges` and `ConvoyClients`. Region names come from the recorded stack's strategic regions; resource names from the shared resource naming.

## Notable behaviour

- The game reads regional efficiency only for countries on a bad naval route, in the regions of its path: where the table was read, a blank cell means no bad route there (never 100% or zero); where it was not read complete the cell is hatched unknown. Days the recording missed are shaded, and an empty chart says "No convoy activity" only for prepared hours whose tables were read.
- A deal's efficiency is the product of its two recorded factors, truncated as the game compares it; the tooltip shows both factors. No per-country figure is made from the deals.
- Lend-lease has no recorded numeric efficiency: its cells are hatched "Efficiency not recorded", with the route's bad flag and convoys in the tooltip.
- Each cell is the day's last recorded read, not an average over the day. At the playhead, the day under way reads the latest recorded hour reached: its cell draws, and its tooltip, pick and drill use that hour. Reads stop at what preparation has accepted: a partly prepared day reads its prepared part, and a day with nothing prepared shows the pending band (its tooltip says it is still being prepared, and it can't be picked), never "Not recorded".
