# Align resource bars with the recorded pools

The balance reply supplies resource identities and presentation order. Fixed5 pool values and the delivered amount of each deal arrive in live cells keyed by native resource index or deal handle. Supply adds domestic production, receipts from subjects and imports. Demand takes project and line needs, transfers to the overlord and the export reserve. Net is what remains after projects less line need. Sort displayed pools by the requested balance key and use authored resource order to break ties and place resources without a row. A hide-empty request omits both unlisted resources and those with two empty bars.

Read the country's pools and the definitions and token spellings. Deals carry importer and exporter in columns, so use the run's owner index to read the relevant deal keys. Read every deal to select the country's when the index is unavailable. Resolve deal partners at the requested hour. The comparison scale asks for every recorded resource by its own index and uses the longest bar across the requested span. No country-wide key-lifetime scan is needed. Return unheld tag spans at reply level and model each resource's missing hours in that resource's result.

The request-versus-panel fixture compares the same structural result and its live cells at each hour. Market purchase progress, resource values and fuel branch amounts can change while the structural panel reply stays the same.

[Resource balance](../../crates/dashboard-economy/src/products/economy/resources.rs) · [Economy request and panel tests](../../crates/dashboard-economy/src/products/economy/tests.rs)
