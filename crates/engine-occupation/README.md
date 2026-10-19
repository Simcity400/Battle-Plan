# engine-occupation

Native occupation readers and their operands, layouts and synthetic tests.

The public capture API is available through [engine](../engine/README.md). Shared rules live in [AGENTS.md](../../AGENTS.md); field contracts, arithmetic, native gates and failure order stay beside each reader.

The module list is in [lib.rs](src/lib.rs). Production dependencies are [engine-deployment](../engine-deployment/README.md), [engine-native](../engine-native/README.md). Tests keep their synthetic fixtures and existing ignored scale gates.

The resistance [hover reader](src/cresistance/hover.rs) passively selects the controller, country-specific state, occupation relation and occupied-country accumulators. It retains live spread caches, exile operands, the compliance-gain ratio, scripted target additions and eligible aggregate local effect values and the child/custom-tooltip counts used by the native nonempty gate. Child contributions are included in the aggregate entries; the occupation description does not print a child tree. No getter refreshes game state.

The hover writer inventory in `cresistance::changes` names resistance integration, script setters, spread and speed refresh, local modifier building, target addition/removal, exile legitimacy and relation-modifier rebuilding. Other patch owners forward country and state accumulator and lifetime changes. No observation invokes a native writer.

The daily diplomacy update is the shared writer of war-goal expiry and exile legitimacy. Occupation owns its single patch; the war-goal observer receives the same diplomacy receiver through a notification held across the original native call.
Garrison template selection visits the country default, every authenticated occupied-country group and live state override. Unset group selections retain inheritance; empty table buckets never establish membership. The template capture acquires this context only after its occupation holder changes.

Template configuration exposes the independent distributable garrison reinforcement priority, authenticated through the occupation holder and read only on its certified changes.

The map-marker reader retains the cached target only for the zero-resistance/nonpositive-compliance suppression edge, and reads the state and occupation-data active definition identities in their stored occurrence order, including inactive objects. Every matching occupation group retains its actual occupied-country pointer identity, which supplies its saved group key and core badge hover scope even when its lists are empty. Its four returned receivers include that same retained pointer separately from the raw occupied reference's resolved country, without another native read. An absent group returns a zero retained receiver and remains distinct from an unreadable key. Definition membership is never reconstructed from levels or aggregate effects. Four active-list writer sites share state certificates and an occupation-holder certificate with the existing membership and lifetime forwarders.
