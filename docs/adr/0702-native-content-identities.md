# Record native content and equipment identities before presentation

Logistics series have one position per equipment archetype. The database assigns the archetype's stockpile index at insertion and preserves it through reloads. Use that index and the database's archetype count for the series, independently of individual equipment definitions.

A country air container is a country's pool registration at one air base. Strategic air is a separate owner. Retain wing pools in registration order, the inherited fixed-point supply ratio, the signed capacity, and the per-base sortie factor. Hourly fuel demand from registered wings supplies the fuel ratio denominator and the allocated base and country fuel supply its numerator. The paired reference candidate starts with zero and has no authenticated semantics in this capture.

Doctrine sharing reads the authenticated upgrade status's companion and its ordered doctrine folder definitions. Resolve those definition identities through the recorded mod stack. Dynamic modifiers used by timed entries and combat tactic definitions share the same native token representation. Read the exact length and storage, validate the terminator, and preserve the resulting bytes without a chosen token limit. The definition's count of value expressions is signed and must be nonnegative.

[Equipment database](../../crates/engine-native-parts/src/cequipment_database.rs) · [Country air container](../../crates/engine-native-air/src/ccountry_air_container.rs) · [Doctrine sharing](../../crates/engine-native-politics/src/cdoctrine_sharing_status.rs) · [Dynamic modifiers](../../crates/engine-native-parts/src/cdynamic_modifier.rs)
