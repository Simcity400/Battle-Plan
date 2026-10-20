# Fuel by branch

## What it shows

Each military branch's fuel consumer for one country (a panel per chosen country): what it requested and received at the latest read of the game's fuel status, and how much of the request went unmet. The View setting switches between Bars (default) and Over time, each branch's received share of its request on a fixed 0–100% scale; the Over time catalogue card adds this widget in that view.

## Products it reads

[`fuel-branches`](../../../../crates/dashboard-economy/src/products/fuel_branches.rs), prepared on its own (family `fuel_branches`) from `CountryFuelConsumers`. Requested and received are the consumer state at the read, exact fixed5 amounts, not a rate and not the completed day's settled receipts (the Fuel widget keeps those). Branch labels are the fuel panel's own localisation keys through the recorded content; consumers come from the recording, in native order.

## Notable behaviour

- A branch that requested nothing shows an em dash ("No fuel requested"), never 0%; a read that was not recorded shows "Not recorded"; hours without a request or a read are gaps on the lines.
- Unmet is the request less the receipt, shown only when positive; a receipt above the request leaves nothing unmet and keeps both amounts as recorded.
- Fuel decimals change only the written amounts. The menu jumps to the supplying read.
- Below 420 px the value column shows the received amount alone and the line under the bar reads "unmet"; both amounts stay in the tooltip.
