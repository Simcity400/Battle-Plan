# engine-deployment

Native deployment readers and their operands, layouts and synthetic tests.

The public capture API is available through [engine](../engine/README.md). Shared rules live in [AGENTS.md](../../AGENTS.md); field contracts, arithmetic, native gates and failure order stay beside each reader.

The module list is in [lib.rs](src/lib.rs). Production dependencies are [engine-native](../engine-native/README.md). Tests keep their synthetic fixtures and existing ignored scale gates.

[unit_bonuses](src/unit_bonuses.rs) reads each country's persistent unit-bonus list from its authenticated deployment status: every entry in stored order with its source kind, id token, number and label, then its exact and category targets with their combat width, nonzero stat words and battalion grants. Requirements, adjusters and the presentation fields the clone rebuild does not apply are not read. A refused object makes the walk unavailable; an entry that does not fit a fixed output is counted with its demand, never stored partially.

Country template configuration reads the selected military-staff template reference and country-wide lock independently of deployment queue membership. The authenticated country census supplies the country; the manager is checked by its vtable and owner backlink. A retained unresolved reference remains a full recorded handle.
