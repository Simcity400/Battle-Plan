# Market ledger

## What it shows

Every recorded market occurrence of the chosen countries' deals (as seller or buyer), newest first: new contracts, requests posted, deliveries, payments collected and contracts removed. Each row has its date, an icon tile in the kind's colour, the title and the seller → buyer line with the equipment, and the exact amount (units or CIC). One toggle per kind (in settings and as filter chips). A row opens Trade flows' contract drill; the menu jumps to its date.

## Products it reads

[`market-ledger`](../../../../crates/dashboard-economy/src/products/market.rs) (family `market`): each occurrence is kept once in its kind's stream, with a dated per-country index. With chosen countries the index names the distinct occurrences where a party had a chosen tag at the occurrence's hour; pages are newest first through the hour shown, only pages in view load, and a page reads only its own rows plus a head per index segment. Every ledger pages by cursor, each page opening with the previous page's `next` (the backend's segment cursor), so the list grows by one waiting page as it scrolls; every country's ledger also names its total, which sizes that waiting page. An empty ledger says no activity only where the market's occurrences were read through the hour shown; otherwise it says some or all hours weren't recorded, or that the hour is still being prepared. `market-lendlease` for the drill.

## Notable behaviour

- A group is asked by its members' dated memberships (aliases included): an occurrence is listed when a party was a member at its hour. Hours the group's membership was not recorded are left out, with a diagnostic; a group recorded with no members says so, and one whose roster was never recorded where it counts says its membership was not recorded; a group is never asked as every country.
- Pages after the first belong to the first page they continue: when preparation advances and the first page changes, they are asked again from its new cursor.
- A delivery whose amount the recording names unavailable reads "Amount unavailable", with its named reason in the tooltip, never 0.
- Lend-lease occurrences belong to Lend-lease agreements, not here. Internal market writer traces (route, pool and inventory mutations) are not ledger rows.
- A removal is not a reason: the ledger never says why a contract ended. An occurrence two chosen countries share is listed once.
