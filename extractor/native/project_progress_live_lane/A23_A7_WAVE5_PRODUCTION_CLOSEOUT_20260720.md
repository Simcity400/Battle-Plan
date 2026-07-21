# A23/A7 wave-five production closeout

Binary authority: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`,
SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

Fresh consolidated exact-a729 export:

- job `20260720T125311Z-w5-a23-a7-owner-route-production-closeou-b01b10dc`;
- executing shard `shard-18`;
- manifest SHA-256
  `53a9a621442fec2faddf6bf4c6068c140e3e44d7f90ccb58fc4add07a8c8b9c4`;
- roots `0x14142F750`, `0x1419F1580`, `0x141431350`,
  `0x1419F22D0`, `0x141961DD0`, `0x1401CAE90`, `0x14220A3D0`,
  and `0x1422083E0`.

## A23 special-project program owner/location

The exact constructor and persistence dispatchers prove two serialized copies
of the same country/tag handle (`CProgram+0x18` and `+0x170`) and province
(`+0x1C` and `+0x174`). Production d65 now takes two equal coherent object
snapshots and admits a row only when both owner copies agree, both province
copies agree, the vtable/program id are valid, and the owner handle exists in
the validated country identity table.

Traversal custody is no longer owner evidence. A CProgram pointer reached
through multiple stale CProgramStatus records emits once under its intrinsic
duplicated owner. A mismatch, unknown owner, torn object, invalid vector, or
conflicting repeat poisons the current rooted walk and leaves d65 unknown for
the hour. The existing pointer-free d65 fold/replay schema does not change.

`tools/program_owner_probe.py` remains the bounded same-hour memory/save
regression acquisition. `tools/verify_program_owner_receipt.py` and
`extractor/native/scripts/Invoke-A23ProgramOwnerFixture.ps1` now verify and
persist its exact 26-row, five-alias, negative-control receipt. The fixture is
not yet executed because the retained corpus has no complete 26-object window.

## A7 convoy terminal route attribution

The production carrier remains d90 CTA1/CTA2:

- full `CFEXMember+0xD0` client handle `{id,type_tag}`;
- identical route key resolved by the exact generic reference registry law;
- concrete producer kind and serializer-proven typed source/destination ids;
- exact d49/d50 combat epoch;
- same-call pre/post D70 mutation proof; and
- gap-free callback producer sequence.

The stalled evidence bridge had two defects: it called the production-blocked
installer and refused route publication while the production control-pair hash
was absent. The collector now:

1. prepublishes a zero-pair, non-promotable resolver snapshot only when
   `BATTLE_PLAN_ENABLE_H_CONVOY_EVIDENCE_HOOK=1`;
2. refuses evidence installation until that snapshot has nonzero member and
   client-route custody;
3. installs the isolated evidence hook, leaving the production receipt and
   activation latch unchanged;
4. drains the old resolver epoch before every republish; and
5. restores/quiesces the hook before the final CTA2 seal and pointer clear.

`verify_h_convoy_fixture.py` plus
`extractor/native/scripts/Invoke-HConvoyFixtureVerification.ps1` authenticate
all raw role hashes and parse their raw JSON/hex windows. They mechanically
join call registers to the member, member to d49/d50 epoch and full client,
client to the exact generic-registry slot, slot to the concrete route object,
serializer fields to CTA1 endpoints, and complete D70 before/after vectors to
each CTA2 mutation proof. The shared baseline, one positive event, zero-event
negative arm, telemetry, final resolver witness, and pair hash remain required.

`H_CONVOY_ROUTE_SOURCE_CLOSEOUT_20260720.json` now binds every retained
exact-20 route-source job to its SHA-256 of `job.json`, semantic request, and
complete artifact manifest (or dump artifact), with decisive serializer and
registry-resolver decompile hashes repeated explicitly. A matching artifact
without that full custody tuple is not route-source authority.

## Irreducible live actions

Static RE, retained saves/streams/captures, fold, replay, UI, hook engineering,
and fixture automation are exhausted. Two state observations remain:

1. A23: one paused exact-a729 process plus `projects.json` from the same raw
   hour, producing the 26/26 owner/province/save equality receipt.
2. A7: one shared-baseline branched positive convoy loss and one no-loss
   control hour, producing the authenticated H-CONVOY pair hash.

Neither action asks a human to identify a field or choose an interpretation.
They only provide gameplay state that does not exist in the retained corpus.

No build, test, compiler, formatter, verifier, or game launch was run by this
subagent.
