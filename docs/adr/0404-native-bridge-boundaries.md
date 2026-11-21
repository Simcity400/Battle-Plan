# Preserve the original call contract at native observation boundaries

The engine site declarations identify observation boundaries. The collector looks up their spans through the authenticated image and forwards the original operation. Changing a site or its span requires checking its receiver, forwarded arguments, return convention, and the instruction boundary where the game resumes. These are read-only observation paths. The ABI notes below use the code's symbolic site names so they can be checked against the declarations and the corresponding collector family.

The site generator infers some names from nearby declarations. The blank lines before the first `end_context` entry and before `DELIVERY` keep those names stable when nearby prose changes. Check the source-derived names before reformatting either section.

## Railway and regional missions

`railway_mutations::MUTATIONS` preserves the original result. `set_province_on_cooldown` and `construct_info` take the province in the second argument. `update_cooldowns` reaches the manager's own list. The save readers and manager-lifetime operations name no individual province. In particular, `read_info` invalidates every certificate because the current definition may not have been installed yet.

| Railway site | Relocated prefix evidence |
| --- | --- |
| `set_province_on_cooldown`, `update_cooldowns` | Saved `RBX` in the fourth argument's home slot. |
| `set_railway` | Stack-pointer copy to `R11` followed by saving `RBX` and `RSI`. |
| `construct_info` | Saved `RBX` in the second argument's home slot. |
| `destroy_info`, `read_info`, `destroy_manager`, `read_manager` | Saved `RBX` in the first argument's home slot. |
| `construct_manager` | Saved `RBX` in the third argument's home slot. |

`regional_mission_mutations::MUTATIONS` observes the country air object in the first argument at both `register_mission` and `unregister_mission`. Each prefix saves `RBX` in the third argument's home slot. The callers do not consume a return value. The bridge still forwards all four register arguments and preserves the original return register.

[Railway sites](../../crates/engine-mutations/src/railway_mutations.rs) · [Regional mission sites](../../crates/engine-mutations/src/regional_mission_mutations.rs) · [Rail observer](../../crates/collector-observers/src/rail_changes.rs)

## Script flags and variables

`script_flag_mutations::MUTATIONS` preserves the original result. `clear_flag` is a frameless entry that tail-calls indexed removal. Its relocated prefix includes the sign-extending read of `cflag_manager::ENTRY_COUNT` and the cleared third argument. `construct_flags` relocates the saved `RBX` and the frame allocation.

`script_variable_mutations::Receiver` selects the store in the first or second argument. `Table` instead names the embedded table in the first argument and uses `TABLE` to find its store. The calling thread's temporary store is selected through `TLS_INDEX_RVA` and `TEMPORARY_STORE`. It is independent of the persisted store selected by the observation. `Patch` distinguishes a relocated entry from a replaced call, which the bridge makes itself. `end_context` takes the first argument from the frame-local context prepared at begin. Every site preserves the original result.

| Variable site | Relocated prefix or context evidence |
| --- | --- |
| `erase_variable` | Saved `RSI` in the third argument's home slot. |
| `read_variables` | Saved `RBX` and frame allocation. |
| `release_variables` | Saved `RBX` in the fourth argument's home slot. |
| `end_effect_command`, `end_effect_command_run` | Console command ends a single stack-local context on either path, after beginning it with the corresponding seed-change option. |
| `end_effect_execute` | Scope-validated effect ends a frame-local context selected through `RBP`. |
| `end_country_daily`, `end_delayed_events`, `end_state_daily`, `end_scripted_window`, `end_fire_event` | Each path ends its own stack-local context. |

[Flag sites](../../crates/engine-mutations/src/script_flag_mutations.rs) · [Variable sites](../../crates/engine-mutations/src/script_variable_mutations.rs) · [Variable observer](../../crates/collector-intel/src/variable_changes.rs)

## Ships, templates and unit leaders

`ship_model_mutations::MUTATIONS` forwards the four register arguments. None takes a stack argument. `RECEIVER_ARGUMENT` selects the object at each entry. `add_carrier_air_base` takes the manager first and the ship second. Base destruction, medal refresh, doctrine refresh, weather refresh and fleet operations select the base, medal store, task force, weather region and fleet, respectively. `reset_reserve_fleet` clears the fleet's leader without reaching `bind_fleet_leader`.

| Ship site | Relocated prefix evidence |
| --- | --- |
| `recalculate_equipment_stats` | Home-slot spill of `DL` and saved `RBP`. |
| `refit` | Home-slot spill of `R9`. |
| `rebuild_experience_modifiers` | Saved `RBX` in the second argument's home slot. |
| `add_carrier_air_base` | Saved `RBX` in the fourth argument's home slot. |
| `rebuild_region_weather_modifiers` | Saved `RBP`, `RDI` and `R12`. |
| `bind_fleet_leader` | Saved `RBX` in the third argument's home slot. |
| `reset_reserve_fleet` | Saved `RBX`. |

`template_mutations::MUTATIONS` preserves the original result. The relocation at `assign_editable_template_data` covers saved `RSI` and the frame allocation. `assign_template_data`, `read_template_data` and `destroy_template` save `RBX` in the first argument's home slot. Assignment preserves the destination and source order. The reader forwards its data, reader and token arguments. Destruction forwards its template and flags.

`unit_leader_mutations::MUTATIONS` is in callback order. The named leader is in the first argument, except for the two exile operations, which supply no leader at entry. The bridge forwards all four register arguments and preserves the original result.

| Leader site | Relocated prefix evidence |
| --- | --- |
| `formation` | Saved `RBX` in the second argument's home slot. |
| `owner` | Owner tag read through the second argument and written to `cunit_leader::OWNER` before return and padding. |
| `exile_removal` | Frame allocation before the second argument is moved into the first. |
| `rebuild_derived` | Saved `RBX` and frame allocation. |
| `operative_rebuild` | Saved `RBX` and frame allocation before the derived-state rebuild call. |
| `operative_admission` | Frame allocation before the conditional test of a receiver field. |

[Ship sites](../../crates/engine-mutations/src/ship_model_mutations.rs) · [Template sites](../../crates/engine-mutations/src/template_mutations.rs) · [Leader sites](../../crates/engine-mutations/src/unit_leader_mutations.rs)

## Territory, world inputs and world lifetime

`territory_mutations::MUTATIONS` uses one owner per native site. A site that names a single receiver supplies it in the first argument. The capture reads `cprovince::ID` or `cstate::ID`, as appropriate. Each bridge forwards up to four register arguments and preserves the original result. `initialize_provinces` may relink provinces from other states. `state_controller` reaches the province notification writer as well, and `reset_state` clears both tags and the custom name and unlinks its provinces.

| Territory site | Relocated prefix evidence |
| --- | --- |
| `province_controller` | Home-slot spill of `R9B`. |
| `victory_points` | Saved `RBX` and frame allocation. |
| `read_province`, `insert_border_conflict`, `erase_border_conflict`, `add_contested_owner` | Saved `RBX` in the first argument's home slot. |
| `rebuild_owner_areas` | Saved `RBX`, `RBP`, `RDI` and `R14`. This site is receiverless. |
| `initialize_provinces`, `state_owner` | Saved `RBX` in the third argument's home slot. |
| `state_controller` | Stack-pointer copy to `RAX` and indirect save of `RBX` in the third argument's home slot. |
| `reset_state` | Saved `RBX` in the second argument's home slot. |
| `read_state`, `state_name_effect` | Stack-pointer copy to `RAX` and `R11`, respectively, followed by saving `RBX` in the first argument's home slot. |
| `remove_contested_owner` | Home-slot spill of `EDX`. |

| World input site | Relocated prefix evidence |
| --- | --- |
| `rename_province` | Saved `RDI` and frame allocation. |
| `reset_province_name` | Saved `RBX` in the second argument's home slot. |
| `set_region_access_modes` | Saved `RBX` and frame allocation. |
| `reset_navy` | Saved `RBX` in the first argument's home slot. |
| `update_threat_total` | Saved `RDI` and `R15` and frame allocation. |

`world_lifetime_sites::CONSTRUCT` lies after base setup and before derived ownership and singleton publication. No incoming branch or call return enters its relocated instruction. `STATE_RELOAD` is the state editor's apply/save reload. It reparses the state-template database and rebuilds the state objects and their template pointers. It supplies the only post-load writer to the templates' base resources. The first argument is the editor. The prefix saves `RBX` in the second argument's home slot.

[Territory sites](../../crates/engine-mutations/src/territory_mutations.rs) · [World input sites](../../crates/engine-mutations/src/world_input_mutations.rs) · [World lifetime sites](../../crates/engine-mutations/src/world_lifetime_sites.rs)

## Retained naval results and peace settlement

`naval_result_sources::NEW` observes the fill call after result identity is assigned. No branch enters that call and the observer preserves its original frame. `MERGE` calls with the existing result in the first argument and returns to the original epilogue. Its span stops before the next function. Use `TARGETS` for original calls and resolve mid-function continuations separately. `CONSTRUCTOR` receives the complete combat object in the first argument and returns that object. It relocates the `RBX` save in the caller's home space. The observer can supply the caller's return PC as its second argument while preserving the original receiver.

`peace_finalizer_site::RVA` follows the finalized-flag write and precedes destruction of turns and actions. Both incoming paths hold the conference in `R13`, including immediate civil-war and zero-factor outcomes. The caller already registered the full reference. The observation has no player or country filter and does not depend on the hourly conference roster. Retained action pointers require independent lifetime authentication after settlement applies their effects.

The peace bridge jumps through the trampoline to resume the relocated instruction, maintains call alignment and shadow space, and preserves nonvolatile registers. The tail overwrites `RCX`, `RDX` and `R8` before destruction and consumes no other volatile register or condition flag. `RETURN_RVA` identifies the native instruction following the relocated prefix. It has the same stack depth and exception-handler state. Unwind lookup uses it because the observation site's patched tail jump could otherwise resemble an epilogue.

[Naval result sites](../../crates/engine-naval-history/src/naval_result_sources.rs) · [Peace finalizer sites](../../crates/engine-peace/src/peace_finalizer_site.rs) · [Peace bridge](../../crates/collector-peace/src/peace_settlements.rs)

## Production and construction

`line_output_sites::CREDIT` receives the line, signed applied progress and completed count. A resolving organisation handle selects the variant history entry. The collector reads the line before the original call can trigger an upgrade and refresh it. The prefix saves `RBX`. `DELIVERY` receives the country's logistics status, variant and signed amount in units times `UNIT_SCALE`. The prefix spills `R8`. `ROCKET_ADVANCE` receives the line in the first argument and has no relative operand in its entry prefix. The bridge preserves its result even though the daily caller ignores it.

`construction_ic_sites::BUILDING_SPEED` receives the line and output storage and returns that storage. For a non-primary linked line, the value comes from the partner. The prefix saves `RBX`. A subsequent RIP-relative load stays at the original site. `BUILDING_COMPLETE` returns its predicate in `AL` when amount and repair count are both zero. `BUILDING_RESET` clears those operands before ownership transfer rescales levels, so the observer copies the count before calling the original. Its prefix saves `RBX` and reserves the frame before the indirect call.

`RAILWAY_COMPLETE` returns its predicate in `AL` when the railway is cancelled or all route edges reach their target level. `REMOVE_GENERAL_LINE` receives the status and line and destroys the removed line. `RAILWAY_ADVANCE` takes the line and relocates the saved-register prologue. `RAILWAY_PROGRESS` takes the manager, source and destination provinces, signed amount and the country in the fifth argument's stack slot. Its result means the segment completed. The entry relocation preserves the saved `RBX`.

[Line output sites](../../crates/engine-production/src/line_output_sites.rs) · [Construction sites](../../crates/engine-production/src/construction_ic_sites.rs)
