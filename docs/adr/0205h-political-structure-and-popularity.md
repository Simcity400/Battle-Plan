# Certify political structure separately from party popularity

The Politics walk carries the complete result only while structure and popularity certificates both match the last complete read. If structure holds but popularity changed, it refreshes only the held parties' popularity. A structure change forces a full read. This lets a daily update refresh its own fields. Both certificates use whole-domain tokens.

Appearance reset invalidates both domains. Capital assignment and the daily update can leave a structure operand unchanged, so compare the previous capitals or last election with the result and invalidate structure only if the values changed. The capital comparison reads both values together. An occupation can assign the capital the country already has. An unreadable initial operand covers the whole native call conservatively. The daily update holds the popularity guard through the structure comparison. The native owner forwards both shared sites and only an observed interval can certify reuse. These notifications also feed the stability and research input observer.

Forwarding statistics distinguish appearance reset, the daily update, and daily updates that change the last election. The capital setter counts changes to either capital. Tests that need exact tokens use separate watch domains. A test exercising the shared observer asserts movement because other tests can advance or withdraw the same token concurrently.

[Political status observer](../../crates/collector-observers/src/political_status_changes.rs)

## Callback operands and coverage

The `callback!` entries forward RCX through R9 and one stack word, cover the in-flight original, and return its RAX. `notify` receives the site's first operand. For `set_capital` that operand is the country.

| Entry | Operands | Domain |
| --- | --- | --- |
| `set_capital` | Country, state, preserve flag | Structure when a capital changes |
| `set_politics` | Status, party, allowed flag, frequency, date | Structure |
| `force_ruling_party` | Status, ideology group | Structure and popularity via the clamped setter |
| `set_ruling_party` | Status, group | Structure |
| `set_ideas` | Status, removed ideas, added ideas, flags | Structure |
| `add_popularity` | Party, amount | Popularity |
| `set_popularity_redistributing` | Party, popularity | Popularity |

`call_site!` replaces the clamped setter's CALL in the `set_political_party_popularity` and `set_popularities` effects. Each replacement calls the original setter with party and value. The relay then returns to the next native instruction. Both observe the popularity domain. `Hooks::drop` starts a new epoch for the two table-free domains after hook removal and drain.
