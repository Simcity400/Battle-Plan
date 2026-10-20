# International market

## What it shows

A moment view (Follow or Pin, no Live) of the international market, switched by the header's View (also a setting, each view its own catalogue card): Contracts (accepted purchase contracts: seller, buyer, equipment, agreed price, convoys assigned of total, payment collected and completed deliveries), Requests (open requests: buyer, seller, wanted equipment, amount, price level offered) and Inventory (each listed variant: seller, equipment, amount on the market, asking price level). Chosen countries narrow the rows to those they sell or buy in. A row opens Trade flows' market drill for that contract, request or seller.

## Products it reads

[`market-history`](../../../../crates/dashboard-economy/src/products/market.rs) (family `market`), one view per request, looked up locally at the playback date; `market-lendlease` for the drill. Contract terms are hourly; the purchase contract's convoys, collection and deliveries come from its daily read and join only while both parties match.

## Notable behaviour

- A group whose membership was not recorded at the hour shown says so, and a group with no members then says it has none; neither reads as "none" for its countries.
- A request and an accepted contract stay apart. A table that was not recorded at the hour says so ("Market inventory was not recorded.", "Contract terms unavailable."); unknown amounts are em dashes.
- No delivery or payment progress meter is drawn: the recording proves no denominator for one, so the exact recorded counts are shown instead. A request's posting date is not shown, since first being read is not when it was posted.
- An asking price level the seller did not set is an em dash. Columns hide as the widget narrows (deliveries, convoys, collected, then price); their values stay in the row tooltip.
