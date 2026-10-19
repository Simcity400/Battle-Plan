# engine-production

Native production readers and their operands, layouts and synthetic tests.

The public capture API is available through [engine](../engine/README.md). Shared rules live in [AGENTS.md](../../AGENTS.md); field contracts, arithmetic, native gates and failure order stay beside each reader.

The module list is in [lib.rs](src/lib.rs). Production dependencies are [engine-dlc](../engine-dlc/README.md), [engine-executable](../engine-executable/README.md), [engine-native](../engine-native/README.md). Tests keep their synthetic fixtures and existing ignored scale gates.

`market_ledger` reads selected market holders into reusable storage: listed equipment, stored price levels, subsidies with country-list or scripted-trigger conditions, automation options and contract draft/price pools. Its caller supplies the authenticated holder and mutation demand; the reader enumerates no country or contract census and schedules no polling. Partial output after a failed read must be withheld.

Market mutation descriptors cover dedicated inventory, price, subsidy, automation, request, contract, delivery and generated route-item boundaries. Outer delivery/rebuild observers mark contracts for worker reads; nested per-item pool sites remain unpatched. Immutable manifests and the native cumulative delivery state derive purchased whole-unit quantities. Lazy cost-cache operands include live native price factors and base quotes, so a worker can compute an invalid cache read-only without assuming vanilla content or invoking the game.

Fuel mutation descriptors identify the dedicated delivered/lost ADD stores;
their exchange and signed delta are register operands. Request/contract births
publish identities and worker admission only; owned definition readers run for
surviving holders on the acquisition worker.

The market descriptor inventory includes dedicated active-contract removal before
native handle retirement and deferred destruction. Its observer retains constant
final progress/cost operands and identity, with no equipment traversal.
Native retirement moves the former ID into the type word, stores the retirement
marker in the ID word and clears registration before deferred cleanup. The
registration byte distinguishes that form from an active ID equal to the marker.
