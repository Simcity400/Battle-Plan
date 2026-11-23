# 0705. Read radar ownership and railway-gun operands from their own identities

The radar pool is embedded in its country and owns the coverage groups. It retains the raw owner reference supplied at construction. The static-intelligence subobject has a separate identity from the pool's primary identity.

A railway gun has a primary identity and a secondary reference subobject. Its own signed combat-reference count is distinct from the generic unit combat vector and distinguishes the idle and combat actions. Group assignment points to the orders-group assignee subobject, which is the value used for joining counters. The gun's current strength is fixed-point and normalized by its rebuilt template maximum. Manpower is the native signed low word of its storage. Damage caps casualties at that current value, and refill moves country manpower into the gun. The production line uses the shared equipment and production-line fields.

[Radar pool](../../crates/engine-native-parts/src/cradars_pool.rs) · [Railway gun](../../crates/engine-native-parts/src/crailway_gun.rs) · [Railway-gun production](../../crates/engine-native-parts/src/crailway_gun_production_line.rs)
