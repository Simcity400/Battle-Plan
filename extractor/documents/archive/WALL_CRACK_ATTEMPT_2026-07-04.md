# Wall-crack attempt — residual user-fixture items, determined no-user pass (2026-07-04)

Every item previously dispositioned "needs the user" gets ONE more determined
attempt to crack WITHOUT the user, using the matured toolkit (RTTI rediscovery,
save-serializer string xref, disassembly of the display/command handlers, and
whatever oracle actually exists). The point is to shrink the irreducible
user-fixture list to its true floor. For each item: CRACK it (structural proof
or an available oracle) or return a PRECISE reason it genuinely needs the user.

NOT in scope (already owned by other running agents — do NOT touch): C2 game-speed
root + C4 intel-network graph (round-2 crack agent), and all production wiring
(wiring agent). Those two agents are editing tracked source in the same worktree.

Reframe that unlocks several of these: "needs the user" was usually shorthand for
"no SAME-TICK melt oracle for the value." But a field can still be cracked
WITHOUT a value-oracle if it is pinned STRUCTURALLY — i.e. disassembly proves the
field is the operand the game reads to DISPLAY that number or EXECUTE that command.
Attack the display/command code, not just the save.

## Group 1 — Not stored in the save (UI-computed). Attack the tooltip/display code.
- **Exact province remaining-supply.** The map-mode/tooltip computes and caches a
  per-province supply value. Find it: disasm the supply map-mode/tooltip renderer or
  the supply-system per-province update; CProvince is known (id u32 @CProvince+0xA4,
  vtable from naval work). If a per-province supply scalar exists in memory, pin it by
  disasm (the value the tooltip formats). Validation: cross-check against the replay's
  own derivable supply where possible, or structural proof. If it is purely a
  transient recompute with no stored field, that is a CRACKED-as-DERIVED disposition
  (replay recomputes it), NOT a user fixture.
- **Resource flow (produced/imported/exported/used per resource).** The economy
  wrapper region is known (CCountry+0xF68 CProductionStatus, +0xF98 CLogisticsStatus,
  +0xFB8 market entry — see crackqueue_c1). Find the resource-balance container off
  CCountry/CProductionStatus. Partial oracle DOES exist: the melt carries resources +
  trade routes (imports/exports are derivable from trade). Validate the produced/used
  candidates against the melt resource block + trade contracts. 0/8 was "no UI oracle"
  — but the trade/resource melt fields are a partial oracle; use them.
- **Attrition reason (which reason).** The attrition SCALARS are already cracked
  (CDivision+0x600 supply_ratio, +0x5F8 supply_gain). The remaining ask is the reason
  ENUM/flag (why the icon shows). Search CDivision for an attrition-reason enum;
  disasm the attrition-tooltip/icon code to pin which field selects the reason. If no
  stored enum exists (reason is recomputed from scalars), that's DERIVED, not a fixture.

## Group 2 — Single-valued in this save. Pin by TYPE/disasm, not by value-diff.
- **Hub motorization level.** All hubs read level 2, so value-diffing can't isolate the
  field. Instead: disasm the hub-motorization getter/tooltip or the supply-hub struct
  (find the CSupplyNode/hub object; motorization is a small int on it). If the field is
  pinned structurally (the getter the UI calls), it is CRACKED even though every hub is
  2 here. Only if the field cannot be isolated without a differing value does it stay a
  user fixture (B3).
- **Offmap factory total.** Attempt to find an offmap-factory field on CCountry/economy;
  if none exists and the total is recomputed from state buildings + trade, that's a
  proof-backed DERIVED rejection (NOT a user fixture — the earlier B1 already suspected
  this). Confirm which.

## Group 3 — State absent from this save. Pin the field structurally (reads default).
- **Focus-paused bit.** No focus is paused, but the paused bool exists on every focus
  object (reads false). D12 focus_live owns the focus struct. Disasm the pause-focus
  command handler to pin the bool offset; a field proven to be the pause target is
  CRACKED even though all read false here. (This directly answers replay + the master
  contract's focus-paused row.)
- **Convoy / port / disrupted-supply PRESSURE.** The disrupted-supply SCALAR is cracked
  (CDivision+0xA0). The pressure ROWS need a live blockade (save has zero disrupted
  anchors). Attempt: find the convoy/port pressure fields structurally on the
  logistics/convoy objects (CLogisticsStatus @CCountry+0xF98 is a known anchor). If the
  fields are pinnable but simply zero here, CRACKED-structural; if they only
  materialize under a live blockade, NEEDS-USER (B5) — state the which.
- **Live air combat rows + kill attribution / naval convoy-loss attribution.** The air
  combat lane (D42) + naval lifecycle (D49/50) are wired; loss/kill ATTRIBUTION needs
  combat ticks. Attempt: find the combat-participant/kill-credit containers even if
  empty now (RTTI for air-combat/kill objects). If the container is found and simply
  empty (no active air/naval combat this tick), document the structure + defer the
  live-tick validation; if attribution is only computed transiently during a combat
  resolve, NEEDS-USER (live combat).

## Group 4 — Offset found + same-tick-validated; only TRACKING unproven.
- **Division cumulative losses (CDivision+0x668) + current manpower
  (CDivision+0x3D0->+0x4).** Both already match the melt same-frame (value CRACKED).
  The two-poll only confirms they RISE/FALL under combat. Decide: is the same-tick match
  + structural role enough to PROMOTE (value is correct; tracking is a monotonicity
  assumption), or is the two-poll genuinely required? Argue it out. Likely CRACKED for
  value; tracking-confirmation optional.
- **Strategic-redeploy mode field.** Find the SR-state field on CDivision structurally
  (disasm the redeploy command/display, or the movement-mode enum). If pinnable,
  CRACKED; else NEEDS-USER (A2).
- **Garrison child disambiguation.** NOTE: garrison scalars were already cracked in
  wave-3 (army_manpower_need +0x88, strength_ratio +0xF0, garrison_required +0xF8).
  Re-confirm whether B6's ambiguity is already resolved by those; if so, mark RESOLVED.

## Group 5 — Genuinely multiplayer (quick-confirm + defer).
- **Chat/messages, hotjoin/tag-switches (domain 57).** Almost certainly needs a 2nd
  player (the events only exist in an MP session). Spend minimal effort: confirm there
  is no single-player structural route (e.g. a message-log ring buffer that would still
  populate SP), then defer as NEEDS-MP. Don't over-invest.

## Per-item DONE criteria
- **CRACKED** = offset/chain pinned AND validated (available oracle) OR proven
  structurally by disasm (the field is the operand the game reads to display/execute).
- **DERIVED** = no stored field; recomputed at runtime → replay reconstructs it. This is
  a WIN (removes it from the fixture list), not a failure. State the derivation inputs.
- **NEEDS-USER** = genuinely irreducible; give the precise reason (state must exist live
  / value undisambiguable without a controlled change / MP-only) and the minimal action.

## ENVIRONMENT + PROTOCOL
- Worktree, PID 28044, SHA gate 870ebf…, paused D04 1944.2.11.18, melt oracle, python
  primitives, capstone 5.0.7 — same as the crack-queue brief
  (`CRACK_QUEUE_NO_USER_2026-07-04.md`). Use `open_process_with_sha_or_report`.
- DISCOVERY ONLY: write only to gitignored `runs/wallcrack_*/`; touch NO tracked source
  (two other agents are editing it); NO commits/`git add`.
- Codex adversarial gate before claiming ANY crack (Mode A, xhigh, unique -o, verify
  exit 0 + non-empty; retry on the 2-min harness timeout; report honestly on os error
  10013). Fail closed both directions.
- Per item: `runs/wallcrack_<item>/EVIDENCE.md`; final summary = the true irreducible
  user-fixture list (CRACKED / DERIVED / NEEDS-USER per item).
