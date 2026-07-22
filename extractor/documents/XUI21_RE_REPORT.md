# XUI21 extractor RE report — fired events and popups

Status: **IMPLEMENTED FAIL-CLOSED; AUTONOMOUS H-POPUP ACQUISITION OWNER READY**

This is the extractor-owned XUI21 record. It is intentionally separate from the rendering-law report under `replay_interface`. The pinned authority is `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

## Promotion law

The rich fired-event stream is admitted only from the authenticated common fire, popup-materialisation, option-command and timeout producers. Save `fired_event_names` IDs remain an identity-only legacy ledger. Polling, first-observed time, current player, option zero, first localisable description, default picture and global timeout assumptions are forbidden.

The lane-local a729 implementation now performs:

- capture-start enumeration of `CGameState+0x560`, count `+0x56c`, capacity `+0x568`, stride `0x38`;
- exact recipient recovery from the owned `CEventScope*` at pending-row `+0x10`, then tag id at scope `+0x08`; pending-row `+0x18` is actor/originator, not recipient;
- same-call fire tick from `CGameState+0x468`, exact combined-id/static-registry join and monotonically increasing firing sequence;
- rejection of inactive game-state calls and already-fired `fire_only_once` calls before rich-row emission;
- visible option ordinals by exact pointer membership in `CEvent+0x368/+0x374`;
- classification support for all five popup classes by the runtime chrome flags plus the static game-file class;
- player, AI, timeout and no-choice resolution correlation without reproducing weights or RNG;
- exact pre/post pending-vector set difference for instance correlation; numeric
  maximum, first-row and observation-order guesses are forbidden;
- a live equality shadow of the `CGameState+0x538` fired-event hash set,
  including cycle/count consistency checks. A strict game-set superset is a
  missed hook observation, not a passing shadow.

The shared collector installation, d75 emission, recorder fold, typed replay acceptance, and independent A25 title/description/per-option context joins are implemented. The executable classifier rejects any CTextBox owner reused across roles. The Rust and Python consumers atomically reject malformed rich rows, duplicate context ids, invalid chronology/resolution invariants, and any legacy identity row paired with `fired_events_complete=true`. The retained corpus has no raw process-authenticated run that actually exercises all five popup classes plus player, AI, timeout, and no-choice resolution. The collector therefore supplies both coverage-attestation bits as false while keeping the exact hooks installed to collect H-POPUP; `zero_gap()` keeps `fired_events_complete=false` instead of manufacturing completeness. Previously recorded focused-test counts are not a substitute for that runtime receipt.

## Game-file bridge

`extractor/game_defs/compile_fired_event_registry.py` compiles the first-registration, one-based namespace ordinal law and the combined id formula `namespace_ordinal*100000+declared_numeric_id`. The pinned generated registry contains 6,937 mapped events, 434 namespaces and 128 source files. It has zero parse failures and zero combined-id/script-id collisions. Four malformed, nonnumeric BBA script IDs are retained explicitly as unresolved input rather than silently discarded. Oracle `42900001` resolves to `ss_recruitment_event.1`.

Generated registry SHA-256: `a807b4d0294a4b29bf8d82ddbe5cd801ca9bf4870ceef3a16b1b2daf03fd83c4`.

## Exact Ghidra provenance

Every row records function address, job ID, artifact path, artifact SHA-256 and manifest SHA-256.

| Address | Finding | Job ID | Artifact | Artifact SHA-256 | Manifest SHA-256 |
|---|---|---|---|---|---|
| `0x140a028a0` | common fire producer; inactive and already-fired `fire_only_once` calls return before accepted event behavior | `20260717T112230Z-xui21-e1-event-registration-callers-ad130a72` | `C:\Users\Example\ghidra_agent_exports\20260717T112230Z-xui21-e1-event-registration-callers-ad130a72\evidence\func_140a028a0.decompile.c` | `123a7b20e5ce9f06cb000296e75b6495aa7f3a63b377371e26da1e6feaeac5e2` | `a0ddf816afcb3e12c609f334b894a0a0241d48c8f028bf19d99d590776923209` |
| `0x1401cab70` | persistent pending-event writer | `20260717T111722Z-xui21-e1-popup-dispatch-callers-393e8af3` | `C:\Users\Example\ghidra_agent_exports\20260717T111722Z-xui21-e1-popup-dispatch-callers-393e8af3\evidence\func_1401cab70.decompile.c` | `4037300b0d9ecd823897cb4a1e91ec4437f0cd07e44f8cb94c19eb1efd0dd784` | `13b324ece548d2d69cbeda74b480c14dd6724f9057ff551612101a9882fc52c1` |
| `0x14052dd00` | `CEventScope+0x08` is the country tag id, proving pending-row exact recipient | `20260717T111722Z-xui21-e1-popup-dispatch-callers-393e8af3` | `C:\Users\Example\ghidra_agent_exports\20260717T111722Z-xui21-e1-popup-dispatch-callers-393e8af3\evidence\func_14052dd00.decompile.c` | `fbc396844b1d41811e902d70022cb4d2c2ac29f9071dbf70a3abf63e01728d5a` | `13b324ece548d2d69cbeda74b480c14dd6724f9057ff551612101a9882fc52c1` |
| `0x1401dee80` | membership test against the fired-event set at `CGameState+0x538` | `20260717T112230Z-xui21-e1-event-registration-callers-ad130a72` | `C:\Users\Example\ghidra_agent_exports\20260717T112230Z-xui21-e1-event-registration-callers-ad130a72\evidence\func_1401dee80.decompile.c` | `b2241a81a012f4b08829aafdb0e41ba108e6f43da65f5282a77735fa01b70eca` | `a0ddf816afcb3e12c609f334b894a0a0241d48c8f028bf19d99d590776923209` |
| `0x1401edac0` | thunk adds `0x538` to `CGameState*` then preserves `CEvent*` for the set operation | `20260717T221806Z-xui21-fired-ledger-hash-container-v2-470ad744` | `C:\Users\Example\ghidra_agent_exports\20260717T221806Z-xui21-fired-ledger-hash-container-v2-470ad744\evidence\func_1401edac0.instructions.txt` | `9599b94af20a6291b7c89230d74c39974250cc95d8267fedb2e314240fe17b77` | `fa6f031e5d4fed55459b38775d7bb70bbc1c5290f5b1b8d0ee7f69e385fa6846` |
| `0x1401c9d50` | exact fired-set hash-container layout and 16-byte node law | `20260717T221806Z-xui21-fired-ledger-hash-container-v2-470ad744` | `C:\Users\Example\ghidra_agent_exports\20260717T221806Z-xui21-fired-ledger-hash-container-v2-470ad744\evidence\func_1401c9d50.decompile.c` | `ec4936e315e59d2be91fd8223279019c48bb9edc1e1fa151eabdf9f13ed5451b` | `fa6f031e5d4fed55459b38775d7bb70bbc1c5290f5b1b8d0ee7f69e385fa6846` |
| `0x141226000` | popup materialisation, chrome selection and definition-order visible options | `20260717T113029Z-xui21-e1-selected-description-branch-937453d0` | `C:\Users\Example\ghidra_agent_exports\20260717T113029Z-xui21-e1-selected-description-branch-937453d0\evidence\func_141226000.decompile.c` | `2e19db2a0af70124a2bbb4a34e15af315d2313efdccb2f8519a411eb06aff042` | `72488f7fafb6a6a236782b4319ca595be316ca7e7bcce94ee52a175a6fe9095a` |
| `0x14116c180` | whole-function selected-description return tap; caller must own candidate vector `CEvent+0x3d0` | `20260717T140942Z-xui21-selected-description-picture-taps--4095d874` | `C:\Users\Example\ghidra_agent_exports\20260717T140942Z-xui21-selected-description-picture-taps--4095d874\evidence\func_14116c180.instructions.txt` | `79e64b0104206720047c2858371f30f4dff897aca103a044c13d701a49d8a84d` | `9788af7e6b9c42bfa8ae564abbf84c25f1ca489d76c03c25e3a1bd256ade6898` |
| `0x14116b0d0` | event description resolver and `CEvent+0x3d0` candidate owner | `20260717T140942Z-xui21-selected-description-picture-taps--4095d874` | `C:\Users\Example\ghidra_agent_exports\20260717T140942Z-xui21-selected-description-picture-taps--4095d874\evidence\func_14116b0d0.decompile.c` | `fc7ea46413ee47f5aeeec46db82a2bdfb77c756acd0a8c1f3899ce5e1b0df749` | `9788af7e6b9c42bfa8ae564abbf84c25f1ca489d76c03c25e3a1bd256ade6898` |
| `0x141225130` | ordered materialised sprite candidates; exact lookup calls at `0x141225487`, `0x141225837`, `0x141225a0b` | `20260717T140942Z-xui21-selected-description-picture-taps--4095d874` | `C:\Users\Example\ghidra_agent_exports\20260717T140942Z-xui21-selected-description-picture-taps--4095d874\evidence\func_141225130.decompile.c` | `42f78e5a3af3d22faff9fbb6877824db889fceec2f8ad240bfd31d37949ca90d` | `9788af7e6b9c42bfa8ae564abbf84c25f1ca489d76c03c25e3a1bd256ade6898` |
| `0x140a2d0d0` | inner sprite lookup used by the authenticated picture-selection calls | `20260717T141639Z-xui21-sprite-lookup-inner-tap-v2-b728237d` | `C:\Users\Example\ghidra_agent_exports\20260717T141639Z-xui21-sprite-lookup-inner-tap-v2-b728237d\evidence\func_140a2d0d0.instructions.txt` | `251dc0042e2d284341d03a746b15d24030781d951c9cfaae4eab8661efccb031` | `293bcddcce3f901e5273104458357a0be1950d68220699ecc305c04d08eec912` |
| `0x1412249f0` | player option click producer | `20260717T103729Z-xui21-e1-event-option-click-wrappers-da96fb99` | `C:\Users\Example\ghidra_agent_exports\20260717T103729Z-xui21-e1-event-option-click-wrappers-da96fb99\evidence\func_1412249f0.decompile.c` | `7f8f8dfab063c790105e69769767cc120bfffbd35ba1aba300cb7937b3d2125a` | `ad876bf0600df27b384e450a44ea97f4dbe7ca257e7961081ea7a0f9738102bf` |
| `0x141227360` | timeout selection and exact no-option timeout branch | `20260717T103729Z-xui21-e1-event-option-click-wrappers-da96fb99` | `C:\Users\Example\ghidra_agent_exports\20260717T103729Z-xui21-e1-event-option-click-wrappers-da96fb99\evidence\func_141227360.decompile.c` | `4b01f83d59b5f7f78078733dcb213b6394050cc1ffd8acb9818f996b62b34603` | `ad876bf0600df27b384e450a44ea97f4dbe7ca257e7961081ea7a0f9738102bf` |

## Incidental findings retained

- `CGameState+0xa39` is an early active-state gate at the common fire producer.
- `CEvent+0x421` is the `fire_only_once` gate.
- The fired-event set is an in-place container at `CGameState+0x538`: count `u32 +0`, bucket count `u32 +4`, bucket-array pointer `+8`; each node is 16 bytes with `CEvent* +0` and `next +8`.
- The pending record’s actor/originator at `+0x18` must never be substituted for the recipient carried by the owned scope.
- Description selection has single, first-admitted, RNG-selected and fallback paths. The exact output is captured at the function return, not guessed from the authored list.
- Picture selection has multiple ordered candidate call sites. The exact materialised lookup result is required.
- No authenticated unselected-dismiss producer exists in this binary. Window disappearance remains inadmissible as a dismissal.

The final Wave-4 pass removed the last picture-name inference: the three exact
lookups at `0x141225487`, `0x141225837`, and `0x141225A0B` reach
to the picture widget. The runtime now records that first successful key
without equality or suffix matching. It also retires a resolved
`(instance,event,recipient,sequence)` binding only after an authenticated
player, AI, or due-timeout terminal edge; pre-due polls and unobserved window
disappearance retain the binding and cannot fabricate resolution. Fresh
receipt-bound evidence is in
`replay_interface/re_evidence/rendering_laws/XUI21/H_POPUP_AUTONOMOUS_CLOSEOUT_20260720.json`.

## Remaining promotion gates

The executable and game-file questions are exhausted in retained evidence. The
exact-ten replacement receipts are now bound to the three promotion consumers.
One machine-owned promotion execution remains:

The deterministic retained-corpus census is recorded in
`replay_interface/re_evidence/rendering_laws/XUI21/retained_runtime_matrix_census_20260720.json`.
It found zero admissible runtime rows, zero retired cells in the five-class by
four-resolution matrix, and zero retired lifecycle edges. The persisted D04
save candidates resolve to 238 country, 9 news, and 3 unit-leader definitions,
but they lack popup materialisation, resolution, timing, A25 ownership, and
loss/authentication receipts and therefore retire no row.

1. Run `extractor/native/scripts/Invoke-Xui21PopupMatrixAcquisition.ps1`. It
   authenticates the frozen D04 save and release artifacts, seeds a real pending
   instance in a derived engine save, starts a fresh capture process, drives all
   20 class/path cells and four lifecycle edges, restores the active fixture
   file byte-for-byte, invokes the existing strict runtime regrader, and writes
   a create-new immutable join receipt. No human popup interaction remains.
   Only the runtime hook receipt—not the fixture/action declaration—may turn
   the two collector coverage bits true.
The older 20-shard job IDs remain discovery aliases only; they are not the
promotion authority. Ordinary RenderDoc frames or screenshots cannot satisfy
the runtime gate. After the
authenticated fixture is present, centralized Rust validation and a refreshed
pointer-free manifest remain mechanical integration work.
