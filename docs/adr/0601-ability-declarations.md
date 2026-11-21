# Resolve ability sprites and effects from the recorded content stack

The recording carries the ability token. Its sprite comes from the recorded stack when the map resolves that token. Build the default sprite from the token, let an authored icon replace its base, and append `_animated` during finalization. Enumerate direct ability files in database order. Parse each declaration and retain its diagnostics even when the first exact byte name already answers lookups.

Keep ability construction state per declaration. The ability token also supplies the default name and its description suffix. Read its declaration name through the first NUL regardless of quote kind. Display modifiers in country, leader and unit order. Each ability starts in country scope, and later type fields select the scope for the remaining effects. Repeated one-time, hourly and daily effect arrays append using the scope at the time they are read. Read the `allowed` field as triggers and preserve the native reader's handling of ignored and invalid fields.

Share adjusters across an ability's `unit_modifiers` blocks and keep their first encounter order. The parser can create an adjuster for a built-in key or a registered terrain. A repeated nondefault key consumes a token and can close the surrounding block before a following icon. A default-token adjuster reads again, which can also consume tokens a tree parser would assign to the enclosing ability. Keep these boundaries because they change which icon the declaration actually selects.

[Ability reader](../../crates/installed-content-abilities/src/abilities.rs)
