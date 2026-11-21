# 0402. Admit native event observations at verified boundaries

The event readers copy operands before the original game operation can invalidate them. Each observation uses the calling convention and commit point of its own site. The source declarations and the collector's sentinel checks hold the build-specific patch material.

## Equipment destruction

| Symbol | Operand and publication contract |
|---|---|
| `LAND` | The entry takes the army in RCX and returns void. The caller-address wrapper preserves all eight integer slots, including the four stack slots. |
| `ATTRITION` | Replaces one five-byte call after the inactive-consumption, template and loss-roll gates. RCX is the division pool, RDX is the variant and R8 is the positive fixed5 removal. The call returns void and retains aligned stack and home space. Admission rules exclude branch targets and extra PC-relative operands inside the span. Dispatch calls the verified target directly. |
| `AIR` | The void entry takes the wing in RCX, the signed requested count in EDX and the source country in R8D. |
| `GARRISON` | The void entry takes the occupation entry, fixed-point damage and source in RCX, RDX and R8. |
| `DELETE` | Takes game state, unit and reserve-return request in RCX, RDX and R8B. Preserve the complete fourth slot even though the entry leaves it unused. |
| `RETURN` | Replaces a single five-byte reserve-return call with no interior branch targets. The void target receives the reserve manager and division pool. The native route requires a requested return outside naval transfer and a home-area or supply-port connection. |
| `DIVISION_FREE`, `QUEUE` | The destructor takes the primary division in RCX and flags in EDX, returns the original pointer, and releases the pool. Its secondary-base forwarders retain caller identity. `QUEUE` only records cause from its game state and unit operands, so the destructor copies the residual after intervening writers. |
| `PARADROP` | The division, source country, and equipment, manpower and organization divisors occupy five integer slots. The last divisor is on the stack. Division and flooring precede logging. |
| `VOLUNTEER_TRANSFER`, `EXILE_TRANSFER` | The roster-removal call takes the roster country and primary division and preserves the original integer result. The held handle and pool survive removal and remain separately serialized, including for load-restored exile transfers. |
| `SET_EXILED` | The virtual void entry receives the primary division and a boolean. Capture observes scaling and rounding together. The entry changes movement and abilities while keeping active combat. Known division and railway updates do not exhaust the possible load and reset routes. |
| `SCALE_TARGET` | Replace one five-byte call to the void pool and fixed5 factor target. The target performs the scale without nested pool writes. |
| `MERGE`, `HQ_RETURN` | The merge receiver names the survivor followed by its candidate array. It clears signed pools and saturates retained allocations. The five-byte HQ return call precedes queueing the same full division handle. |

`DELETION_CAUSES` classifies the original caller, including the return retained through virtual dispatch, parser tail calls and combat cleanup. `AIR_CAUSES` distinguishes nuclear damage across all wings, accidents associated with the country loss roll and wing accumulator, and strategic bombing or the strategic hourly wing pass. The mission anti-air rows cover rail defenses, mission-dependent losses, configured chance and damage, and the mission's attributed loss flush. Land anti-air is a separate route using night-attack and anti-air operands. Scripted raid-unit damage and the naval exchange channel have their own routes.

## Event feed and experience

`event_feed_sites::WAR` observes the second linkage call. RCX is the status, and the candidate in RDX also survives in RDI while the continuation initializes the war fields. `WarFrame` captures the stack pointer immediately before that call. The entry stack pointer is one return slot lower, and the Rust callback uses its own frame. The dispatch site `event_feed_sites::ANNEX` uses R8 for the primary scope frame slot and a zero R9D.

The `provenance::Caller` keys are returns from complete direct callers of the observed operation. A listed call spans five bytes. `provenance::Entry` detours preserve the native result and up to four register and four stack arguments. Diplomacy dispatch and civil-war callers come from the politics family, and territory forwards state-owner assignment callers. A route outside the admitted lists fails by name. Readers authenticate only the objects they need and copy owned integers.

| Provenance entry | Operand and publication contract |
|---|---|
| `JOIN_WAR` | Takes diplomacy, ally and enemy pointers, a reason and an expansion flag, and returns the war. Expansion follows every enemy of the ally's side. Otherwise it follows the named enemy. |
| `END_WAR` | Takes the linked war and a truce flag before changing both statuses, clearing the link and rebuilding its endpoints. |
| `CREATE_CONFERENCE`, `START_CONFERENCE` | Creation takes manager, loser and winner, a signed factor, two unsigned words, a flag and a message pointer. It stores loser then winner, queues the conference and may start it before returning. The start can immediately end wars and annex losers. |
| `APPLY_CONFERENCE` | Takes a conference pointer and a flag. It can end wars and annex losers left without states. The queue, turn processing and conference-processing tail call share this route. |
| `ANNEX` | Takes the taker, target reference, output pointer, troop-transfer flag and an additional byte. It moves states and ends wars before the separately observed authored dispatch. |
| `RELEASE`, `RELEASE_COMMAND` | Both pass releaser and target pointers and traverse core states. The direct operation takes two byte arguments, and the command takes one and returns a byte result. |

`experience_gain_sites::SITES` uses RBX for status and RDI for adjusted amount, with RSI holding the amount on the air path. The seven-byte prefix ends at the bank addition. Entry branches reach its start, and none land inside or return within it. Preserve integer registers, flags and vector registers. The daily floor store belongs to the continuation.

The incoming diplomacy reader expects the native array header in data, capacity and count order. The wrapper's identity copy spans its vtable, complete handle, status byte and body pointer together so a partial identity cannot slip through.

## Map notifications and operation outcomes

| Symbol | Operands and validity |
|---|---|
| `MAP_MODE_SINGLETON_RVA`, `INTEL_STRIDE` | The selected mode is the singleton's first word. A unit intel entry contains a canonical country ordinal and a signed `INTEL_LEVEL`. |
| `FAKE_REVEAL_VISIBILITY_CALL_RVA` | The local intel query receives the fake division in RCX after the queued-deletion gate. |
| `RAILWAY_OVERRUN_RVA` | Receives the gun, its province and the destination province in RCX, RDX and R8. Text belongs at the destination. |
| `CRITICAL_BOMBARDMENT_TAP_RVA` | R13 retains the bombarded province just after the critical branch's draw and before the building list is used. |
| `GENERAL_CAPTURE_CALL_RVA` | Receives the leader, captor reference and province in RCX, RDX and R8. A null province supplies no text. |
| `PARADROP_LANDING_CALL_RVA` | Receives province, owner reference and the landing-loss byte in RCX, RDX and R8B. |
| `RESISTANCE_ACTIVITY_RVA` | RCX names the state's resistance object and RDX names the activity definition. |
| `BUILDING_NOTIFICATION_RVA` | Receives building, change source, level and previous health in RCX, RDX, R8W and R9. The source starts with the reason word. The next byte operands admit the client branch and the text. |
| `operation_outcome_sites::COMPLETION_RVA` | The five-byte stack-spill span admits no relative operand, return point or interior target. Further bytes verify the entry without extending the patch. |

The strike entries `nuclear_strike_sites::{IMPACT, NOTIFY, UNIT_DAMAGE}` take strike and province in RCX and RDX. Impact's five-byte span preserves its second argument in the home slot. Notification is entered only by the impact's tail call, retains operands in RSI and RDI, and saves RBX in its five-byte span. Unit damage preserves the original 64-bit return and the nonvolatile registers covered by its five-byte span. Both province-damage branches visit occupants in native order. The unit walker skips the excluded class and uses the reach's loaded strength and organization bounds for the immediate-damage operands.

[Equipment](../../crates/engine-events/src/equipment_destruction.rs) · [Event feed](../../crates/engine-events/src/event_feed_sites.rs) · [Provenance](../../crates/engine-events/src/event_feed_sites/provenance.rs) · [Experience](../../crates/engine-events/src/experience_gain_sites.rs) · [Map events](../../crates/engine-events/src/map_event_sites.rs) · [Strike](../../crates/engine-events/src/nuclear_strike_sites.rs)
