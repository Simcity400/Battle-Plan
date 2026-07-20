# XN14Q weather update/setter-chain promotion request

Status: **fresh exact-20 static receipts bound; consumers wired; live proof remains closed**

Pinned authority: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

The machine-readable request is `replay_interface/re_evidence/rendering_laws/XN14Q/weather_update_setter_chain_promotion_request.json`; the resulting implementation record is `replay_interface/re_evidence/rendering_laws/XN14Q/automated_selected_present_wiring.json`. The Wave 3 exact-20 binding ledger admits the static function and data receipts. Those receipts authorize the static implementation work below, but do not by themselves authorize active-weather promotion; the authenticated live-source gate remains separate.

## Bound exact-20 consolidation seed

Submit one `export_functions`, depth `1`, p-code enabled request for:

- `0x14166ec50`: weather membership/visibility/playback producer;
- `0x14166cec0`: normal per-update owner;
- `0x14227b200`: active setter;
- `0x142271b40`: alpha setter;
- `0x14227b4a0`: playback setter.

The canonical request SHA-256 is `d96e1b37a4087b19507cefe96823b8f3438afee91bfe9573870322b2dd22daf8`. A separate three-address `dump_data` request for the setter prologues has canonical SHA-256 `792a4380f12fa38f6914750f8973085a8932e9c95e3fbd158edaf6c9084579e7`.

Depth one must retain `0x1422c2ba0`, `0x1423e9160`, and `0x1423e9170`. Those children prove the propagated active bit and the exact attached-child alpha/playback storage at `+0x4c` and `+0x50`.

## Static result

The producer reads the exact scalar from `view+0x184`. Playback is written to both members of every live pair on every active call. Fading active is written in all five camera branches; fading alpha is called only when computed alpha is positive. Primary active is written only in far-fade/far-hidden; primary alpha only in far-fade. Therefore the current portable and extractor validators are too coarse: they equate active and alpha write occurrence and incorrectly expect an alpha call in hidden branches.

No retained artifact supplies a runtime occurrence row for those calls. The static result defines what the hook must observe and reject against; it does not manufacture a runtime write mask.

## Production closure after exact-20 static admission

The implementation sequence below is now wired in the extractor, portable validator, selected-Present receipt, and collector fold. This is an implementation state, not an activation receipt:

1. Add the three authenticated setter detours to the weather staged-hook group.
2. Assign update ownership at the normal `0x14166cec0` caller and separately tag editor refresh.
3. Emit the first successful region call even when code zero is unchanged; reject an unseeded late-active first observation.
4. Record setter occurrences only while the exact weather producer call is current, then double-read post-update root/attached-child state.
5. Bind rows to the same PresentEnd lock and include their digest in candidate selection.
6. Fold region epochs in the collector, erase pointers through the authenticated weather catalog, and join child effects to the selected submit's complete ordered CPU packet cover.
7. Admit an empty effect-output subset only when the complete authenticated packet list proves that exact empty set.

The exact-20 static receipt is bound and the non-human consumers are wired. Promotion remains closed until central Rust validation succeeds and one same-session early-launch production observation proves the transaction. Human input is not required for the static seed, ABI, or implementation design; it is required only for that final controlled live observation.


Normal-owner data job `20260720T051448Z-w4-weather-normal-update-owner-prologue-8da0604f` (artifact SHA-256 `5bd70ceeedea688cb4df374eba85c1bc880f9970adca68a24f8c9bdb3cabf3ac`) confirms entry bytes `40 57 41 56 41 57 48 83 ec 50 48 89 5c 24 78 4d`. This corrected a shifted draft sentinel that omitted the leading `0x40`; without the correction the authenticated hook installation would always reject the pinned binary.
