# 0402. Keep staged operands tied to the writer that commits them

Entry detours copy inputs while their objects are live. Interior taps identify a reached operation and may stage a witness for the enclosing writer. Publication follows that writer's committed store or verified return, preserving native result values and leaving rejected branches silent.

## Power and politics

| Symbol | Operand and commit contract |
|---|---|
| `ADD_POLITICAL_POWER`, `ADD_POLITICAL_POWER_NO_DEFICIT` | Take status and signed fixed5 in RCX and RDX. The ordinary signal follows clamping. The no-deficit entry applies its native floor. |
| `SET_POLITICAL_POWER`, `SET_POLITICAL_POWER_EFFECT` | The effect enters with effect and scope pointers in RCX and RDX. Its interior tap stages status in RSI and evaluated target in RCX. The evaluated store and conditional zero floor both precede the enclosing return. |
| `CLEAR_POLITICS`, `RESET_COUNTRY` | Clear takes status in RCX and retains the old government before party destruction. Reset takes country in RCX and observes command-power reset separately from nested clear and setup. |
| `ADD_COMMAND_POWER` | Takes country and pointer to signed fixed5 and returns void. |
| `SPEND_COMMAND_POWER`, `ALLOCATE_COMMAND_POWER` | Spend takes country and a charge pointer. Allocate takes country, allocator and an R8B charge byte. Both use AL for success and preserve full RAX. |
| `RELEASE_COMMAND_POWER` | Takes country and allocator. The nested refund uses its own cap. |
| `ACTIVATE_ABILITY` | Takes country, definition, scope and charge byte. Upfront subtraction precedes nested allocation. |
| `ABILITY_UPKEEP` | One paid anniversary uses country in R13, balance in RCX, charge in RDX and the sum of R15 and RBX to name the record. |
| `GROUND_CREW` | The four register arguments name country, region, allocator and adjustment. The fifth stack slot supplies the charge byte. |
| `SET_RULING_PARTY`, `FORCE_RULING_PARTY`, `SETUP_POLITICS` | Commit pairs are RBX and RAX, RBP and RSI, or RBP and RCX for status and new party respectively. Set and force reach their commits before refresh and on-actions, and setup also handles country reuse. |
| `FALLBACK_RULING_PARTY` | R14 is status, and its stored party is authoritative at the loop exit. The zero flag distinguishes exhaustion or an empty roster from the branch reusing an existing party. Intermediate maxima are silent. |
| `DAILY_POLITICAL_RATE` | R13 is status, RDI gross, stack reduction, RAX net and R15 blockade count. Observe before tooltip adjustment. |
| `DAILY_PP_MODIFIER` | Reads reduction after the cost call from the unchanged caller, R13 status and stack cost. Its seven-byte span contains no relative operand, return point or interior entry. |
| `PP_SIGNAL` | Observes after the signal's assertions. Its five-byte span restores stack and register state and admits no relative operand or interior entry. The flush reads thread-local state and leaves the signal in RCX unused. |

`CHARGING_CALLERS` identifies return routes independently of item identity or table differences. `SCOPES` preserves up to five integer or pointer arguments and the original RAX result.

## Script events and lifecycle sites

| Symbol | Operand and commit contract |
|---|---|
| `ACCEPTED` | Runs after world and fire-once admission. The definition and recipient are in RSI and R13. Actor and bound scope are in the home slot and fifth argument slot. |
| `EXISTING_INSTANCE`, `OPTION` | The pending instance or its absence is in EDI while the definition and frame are live. `OPTION` runs after both ordinal gates with event in RBX, scope in RSI and admitted ordinal in EDI. |
| `REGISTER` | Preserves all six original integer or pointer arguments. The native instance occupies the low word of RAX. |
| `DELAYED_FIRE_BOUNDARY` | Runs after a decrement reaches fire evaluation. RSI is ordinal, RBX timer and RDI owner. Quiet timers bypass it. |
| `ERASE_FIRED` | Runs when replacement unlinks a fired node. RSI names the world and RDI names the live old definition. A no-match route bypasses the count store. The witness is published by `REPLACE_EVENT_RETIREMENT_CALL`. |
| `MISSION_APPENDED`, `MISSION_REACTIVATED` | Append is observed after the original return, with RBP status, RSI definition and R12D days. Reactivation stages a witness for `AUTO_MISSION_ACTIVATION` before duration and active-state stores. |
| `TARGETED_MISSION_REENABLED`, `TARGETED_DECISION_REENABLED` | Timed cooldown expiry supplies RBX instance and RBP status before reenable. Ordinary expiry has RSI status, RBX instance and zero in R12D. Its store and terminal jump form one admitted span. |
| `MISSION_COMPLETED` | RSI status and RBX timed instance supply the stored result before effects, which may reactivate the mission. |
| `DECISION_COOLDOWN_FINISHED`, `REFRESH_COOLDOWN_FINISHED` | The selected cooldown in R8 is still live immediately after membership removal. RBP names ordinary status and R14 names refresh status. |
| `TAKEN_DECISION_REMOVED` | RDI definition and RSI status survive membership removal before optional effects. |
| `MISSION_REMOVED`, `MISSION_CANCELLED` | RSI status and RBX instance survive the post-effect reactivation check for removal or the already stored duration for cancellation. |
| `TARGETED_DECISION_REMOVED`, `TARGETED_MISSION_REMOVED` | RBX names the instance after state and duration stores. The ordinary targeted status is in RSI, and the timed status is in RBP. |
| `TARGETED_MISSION_CANCELLED` | RBX instance and RBP status are live for the duration store and terminal jump after the state store. |
| `TARGETED_MISSION_COMPLETED`, `TARGETED_MISSION_TIMED_OUT` | RBX instance and RBP status remain live before effects. ECX supplies the admitted completed or timed-out state. |
| `EXTEND_MISSION_TIMEOUT`, `EXTEND_DECISION_REMOVAL` | The retired pre-add descriptors held mission, scope and signed days in RDI, R15 and EAX, or removal timer, scope and effect in RAX, RSI and RBX. The enclosing call descriptors now supply publication. |

## Enclosing decision writers

`decisions::call_span_len` stops a selected call's span at the boundary of the adjacent patch where applicable.

| Symbol | Operand and commit contract |
|---|---|
| `AUTO_MISSION_ACTIVATION` | RCX is status. It can skip, append or reactivate each retained mission, with reached duration and active-state stores complete before returning. |
| `TIMED_SELECTION` | Takes status and definition. It stores the selected timed instance's active state and days before returning the zero-extended result in RAX. |
| `TARGETED_IGNORE_COMMAND`, `IGNORE_ALL_COMMAND` | RCX names the command. The original completes the selected or admitted ignore stores before returning. |
| `TARGETED_REMOVAL_REQUEST` | RCX effect and RDX scope select either lookup. Both lead to the same committed removal-request store. |
| `CANDIDATE_FINALIZATION` | RCX names status. Every one of the four candidate headers is cleared before the original returns. |
| `EXTEND_MISSION_TIMEOUT_CALL`, `EXTEND_DECISION_REMOVAL_CALL` | RCX names the instance or timer and EDX the signed day delta. Scope is in R15 for mission timeout and RSI for decision removal. The call completes the scalar add before returning. |
| `REPLACE_EVENT_RETIREMENT_CALL` | RCX world and RDX old definition identify the replacement. A fired match unlinks a node and decrements the count before the return. A miss supplies no retirement. |

[Power and politics](../../crates/engine-events/src/politics_sites.rs) · [Script event sites](../../crates/engine-events/src/script_event_sites.rs) · [Decision writers](../../crates/engine-events/src/script_event_sites/decisions.rs)
