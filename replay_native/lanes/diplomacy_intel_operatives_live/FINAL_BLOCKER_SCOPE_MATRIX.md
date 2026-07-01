# diplomacy_intel_operatives_live final blocker scope matrix

Domain: `53`
Current accepted source mask: `0x7F`
Current validated live roots: country, relation/opinion/flags/subjects, agency,
cryptology, operative identity/stats, and war row/casualty root.

War is no longer a root blocker. Current `870ebf` war rows are validated as a
global row pool keyed by tag pair, with current date RTTI vtables
`CGregorianDate module+0x2702190` and `CGameDate module+0x2702388`,
casualties at `+0x40/+0x48`, and echo tags at `+0x68/+0x6C`. Remaining war
scope is lifecycle detail only: embedded date scalar decode and higher-level
war grouping/name.

## Blocker Matrix

| Blocker | Current status | Positive fixture required | Rejection rule | Compact behavior |
| --- | --- | --- | --- | --- |
| `operation_detail` | Current operation RTTI definition surface is pinned, but no running-operation root/fields are accepted. Old `16022851` operation RVAs are rejected by RTTI and heap scan. | Same-tick `870ebf` running operation with `owner_tag`, `target_tag_or_state`, `operation_token`, `operation_name`, `phase_or_progress`, and `assigned_operative_ids`. | Definition-only databases, `COperation` heap presence without owner/target/progress/slot decode, and stale RVAs are rejected. | Keep operation anchors resisted and set `STATUS_OPERATIONS_DETAIL_RESISTED`; emit no unresisted operation rows. |
| `intel_map_percentages` | Save oracle has positional `countries.<TAG>.intel.intel`; no live owner/target percentage root is accepted. | Same-tick viewer/target pair with visible or save-corroborated civilian, army, navy, and air percentages. | Reject unkeyed value multisets and positional-only save rows; candidate must bind viewer and target in live memory and warm-revalidate all four lanes. | Keep intel-map rows resisted and set `STATUS_INTEL_MAP_RESISTED`; emit no unresisted percentage rows. |
| `incoming_diplomatic_action_queue` | Validated live surface is settled relation state/flags only; no pending-action queue root is accepted. | Same-tick visible incoming diplomatic action with `actor_tag`, `target_tag`, `action_kind`, and visible queue evidence. | Reject settled relation flags, modal definitions, and accepted/expired actions without a pending queue root. | Set `STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED`; emit no incoming-action queue rows. |

## Gate

Use `remaining_root_fixture_gate.py` with the current war proof and operation
proof. The gate exits `2` until all three blocker sections above have accepted
current-build evidence. Fixture template:
`fixture_contract.template.json`.
