# recorder-oracle-construction

Construction quantities, progress, output, target levels and occupation inputs. Owns [construction.rs](src/construction.rs) and its child tests. The [oracle command](../recorder-oracle/README.md) composes comparisons; this crate calls its specific dependent comparisons directly.

Table completeness, exact joins, differences and named limitations use [shared oracle support](../recorder-oracle-support/README.md). Content-dependent facts resolve through the recorded content stack. Synthetic fixtures are dev-only through the support crate's `test-support` feature.

Shared rules live in [AGENTS.md](../../AGENTS.md); the [recorder guide](../recorder/README.md) defines fresh-save/load and matched-native-observation checks.

Resource-map comparisons include native own/rights-holder origin order and outgoing lend-lease order from the save. The writer sorts resource rights by resource name, so their native order is runtime-only, along with received lend-lease, directional market contract and supply transfer client lists. Required positions and uniqueness are checked without claiming saved order equality; saved resource-right membership still compares exactly.

Market-referenced delivery routes' source/destination states compare exactly with the seller's saved `resources.delivery_routes` entry for the buyer. Contracts join by their saved seller/buyer and convoy-client handle; captured membership, route country/class and freshness are checked separately. Missing or different endpoint values fail. Only a seller/buyer pair with no serialized route entry has a named runtime-only limitation; present saved differences are never waived.

Delivery-route keys reconcile in both directions with every current client's reference, including imports and lend-lease. Shared references need only one route row; missing routes and unreferenced recorded routes are differences.
