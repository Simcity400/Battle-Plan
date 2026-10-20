# Lend-lease agreements

## What it shows

What each chosen country sends (or, with the Direction switch, receives) through lend-lease: one row per agreement with the partner, the equipment, the game's schedule forms (Monthly, Production share, Once), the delivered and lost totals the agreement retains, the last delivery and the active-since date. Fuel terms get their own row in their own units. Ended agreements stay, muted, unless the Ended agreements setting hides them; they show their last recorded values. A row opens Trade flows' agreement drill at the drill date.

## Products it reads

[`market-history`](../../../../crates/dashboard-economy/src/products/market.rs) in its `agreements` view (family `market`), with `market-lendlease` for the drill. Equipment amounts are fixed5, fuel quantities /32768 and production shares fixed5 fractions shown as percentages. Equipment names come from the variant catalogue at their naming reads; art from the stockpile artwork product for mounted rows only.

## Notable behaviour

- A group whose membership was not recorded at the hour shown says so, and a group with no members then says it has none; neither reads as "none" for its countries. Membership still being prepared shows that, never either.
- Delivered and lost are the agreement's retained totals, which the game may reset on an amendment; they are not a lifetime shipment ledger. An item not in a pool is an em dash, a recorded zero is 0.
- No delivery efficiency is shown: the game keeps no numeric lend-lease efficiency. The tooltip's share lost is lost over delivered plus lost of the retained totals.
- Agreements match a country by each side's tag at the read, so a reused tag never joins an unrelated holder. Columns hide as the widget narrows (Since and Last delivery first, then Lost); their values move into the row tooltip.
