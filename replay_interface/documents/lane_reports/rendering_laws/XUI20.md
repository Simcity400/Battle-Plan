# XUI20 — combat detail, losses, logs, and war overview

Date: 2026-07-18  
Disposition: **REPLAY-OWNED SURFACES PROMOTION READY; EXTERNAL EXTRACTOR INPUTS FAIL CLOSED**

## X2 combat-panel closure (2026-07-20)

Land-combat side leaders and effective displayed modifiers now come from the
game-driven `0x1415C9860` materialization scope. Nested `0x14129E190` leader
selection and `0x1415BBF00` modifier appends are carried pointer-free through
D55 v8. Replay admission requires source
`authenticated_a729_combatview_materializer_1415c9860`, a positive capture
sequence, exact/exact-none leader semantics, and pinned engine modifier order.
A panel not refreshed during capture remains
`unavailable_panel_not_observed`; side-flag reconstruction is forbidden.

## Replay promotion result

The replay layer now consumes and independently admits every exact XUI20 family carried
by typed recording data:

- active land identity, participants, terrain, width, progress, all retained d55
  lifecycle leaves, ordered participant events, both complete side rows, tactics,
  counts, damage, loss status, and attribution;
- active naval identity/generation, location/region, side counts and positioning,
  task-force ids, exact side-country sets, the complete ordered member roster,
  and fail-closed SHP1-v2 terminal sunk-ship history admission;
- air wing loss history, exact count/equipment arithmetic, air-region aggregate rows,
  every typed damage-cause row, and sunk-convoy history;
- D60 pair history and legacy relation casualties without treating either as a complete
  war object;
- W1A/W1B engine-resolved war name, front/start identity, ordered nonempty sides,
  ordered participant rows, participation, all nine contribution members,
  capitulation progress, and signed overall progress;
- O1's exact disposition: producer-family order is preserved and a nonexistent global
  cross-family sequence is explicitly `NotApplicable`.

`admit_xui20_replay` returns only `Admitted`, `ExternalInputRequired`, `Rejected`, or
`NotApplicable`. Missing measured leaves never become zeros, source keys never become
war display names, D60 pairs never become W1 rows, and disappearance never becomes a
terminal result. The aggregate combat-log admission also rejects or fails closed when
any constituent producer family does.

The fixture generator now emits the authoritative integer convoy equipment type `70`,
not the stale string `"convoy"`. Fixtures and their manifest were regenerated through
`replay_fixture_gen`. During that normal regeneration, the same warning gate proved the
XUI17 fleet formation id must be integer `500`; task-force uid `501:1` remains distinct.

## External extractor inputs only

These are not replay implementation gaps. The consumers and explicit admission states
are ready, but production cannot admit the affected surface until the extractor emits:

1. the terminal land-record owning-vector join for the exact `0x448` result/loss row;
2. one authentic measured-empty-ledger naval-sink activation receipt for the
   already integrated three-wrapper SHP1-v2 producer;
3. the scheduled W1A/W1B war-object payload already modeled by replay;
4. W1C participant division count, participant IC, ordered live wargoals, and terminal
   `war_result`;
5. active-panel leaders, effective displayed modifier stack, bombardment selector, and
   air-superiority selector where those widgets are requested.

M02 is an on-map combat-indicator capture packet, not an XUI20 panel extractor input or
promotion blocker. A global combat-log order is not data-blocked: the exact engine law
proves it does not exist.

## Mechanical evidence

The replay-only verifier runs the focused XUI20 parser/binding/admission tests and the
complete `replay_core` and `game_ui` suites. Two independent receipts are required.
`build_xui20.py` hashes replay sources separately from the frozen extractor producer-law
authority, then emits the production contract and evidence manifest. Extractor code and
`PROGRAM_STATUS.md` are outside this packet.

Binary authority: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

## Ghidra provenance

| exact law | function address(es) | job ID | artifact path(s) | manifest SHA-256 |
|---|---|---|---|---|
| active land vtable neighborhood | `0x141295984`, `0x1424a9cf0`, `0x1412a8750`, `0x1424a9730`, `0x1412a5230`, `0x14011cd10`, `0x140129db0`, `0x1412a3ef0`, `0x142209a50`, `0x14116d9c0`, `0x140120050`, `0x1401207a0`, `0x1401241a0`, `0x1409f6070`, `0x14066a860` | `20260717T105015Z-xui20-a33-land-combat-vtable-bounded-v1-6852ed23` | `C:\Users\Example\ghidra_agent_exports\20260717T105015Z-xui20-a33-land-combat-vtable-bounded-v1-6852ed23\datdump.txt` | identity manifest `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |
| terminal land row and visible ordering | `0x1416d9790`, `0x1416e07c0`, `0x1416d4b00`, `0x1416e0390` | `20260717T141034Z-xui20-a33-combat-log-ui-consumers-v1-90f0dd7a` | `C:\Users\Example\ghidra_agent_exports\20260717T141034Z-xui20-a33-combat-log-ui-consumers-v1-90f0dd7a\evidence\func_1416d9790.decompile.c`; `func_1416e07c0.decompile.c`; `func_1416d4b00.decompile.c`; `func_1416e0390.decompile.c` | `e2c5ade5124625cfdacb24b2298d19de1f5b3509afd9d2a40f573ef6e6889a15` |
| equipment-loss materializer | `0x1416debe0`, `0x1416de940` | `20260717T141644Z-xui20-a33-land-equipment-loss-consumer-v-4fb27f64` | `C:\Users\Example\ghidra_agent_exports\20260717T141644Z-xui20-a33-land-equipment-loss-consumer-v-4fb27f64\evidence\func_1416debe0.decompile.c`; `func_1416de940.decompile.c` | `ab60615d5497a1bba0b0d10ba7957df89817409a3bf268cda5f38accc6f1f663` |
| separate log-family vectors | `0x140cd0fc0` | `20260717T140941Z-xui20-a33-orders-group-logs-vtable-funct-ad528abc` | `C:\Users\Example\ghidra_agent_exports\20260717T140941Z-xui20-a33-orders-group-logs-vtable-funct-ad528abc\evidence\func_140cd0fc0.decompile.c` | `16468e2e726703d855ebff14fb65a8eb8a6d5dc98177c4c4ce4c240f76bf5730` |
| sunk ship layout | `0x140c1f090`, `0x140c1ebc0` | `20260717T140953Z-xui20-a33-sunk-ship-vtable-functions-v1-72ed4c05` | `C:\Users\Example\ghidra_agent_exports\20260717T140953Z-xui20-a33-sunk-ship-vtable-functions-v1-72ed4c05\evidence\func_140c1f090.decompile.c`; `func_140c1ebc0.decompile.c` | `755a0011376d06bbd38456a2bb009236d99f4481a9b076789d9d57729c5673fd` |
| sunk ship date producer and item consumer | `0x140c1e960`, `0x141bddef0` | `20260720T102819Z-w5-a33-terminal-sink-chain-core2-e794ec80` | `<local-path>`; `func_141bddef0.decompile.c` | `466880cc290e8961dab48c8323346d9d862089d85bbe0f4ba285b924cc959c82` |
| all global post-append writer routes | `0x1401d2c90`, `0x1401d2d10`, `0x1401d2da0`; excluded temporary `0x140c23290` | `20260720T111238Z-w5-a33-all-sunk-record-producer-routes-c6e0fa81` | `<local-path>` | `d02f0f1aaf2369db9dc9529062f5e8f59f3f6f751fb9e4c0ab81d47dea1e1de0` |
| sunk convoy layout | `0x140e9e850` | `20260717T141001Z-xui20-a33-sunk-convoy-vtable-functions-v-8177d138` | `C:\Users\Example\ghidra_agent_exports\20260717T141001Z-xui20-a33-sunk-convoy-vtable-functions-v-8177d138\evidence\func_140e9e850.decompile.c` | `4a9ca0aeda1700055809fee1b827f4ad7379108208cba6c4fdeeddeb00dbdab3` |
| exact air-region storage/selection | `0x141950340`, `0x14194f590` | `20260716T225947Z-d42-airregionstatistics-enum-and-damagee-adfdde5a` | `C:\Users\Example\ghidra_agent_exports\20260716T225947Z-d42-airregionstatistics-enum-and-damagee-adfdde5a\evidence\func_141950340.decompile.c`; `func_14194f590.decompile.c` | `9e085a96163dfea76f644978faafe552bc7301e4bc064aa0aadd63b93320969b` |
| war identity and ordered sides | `0x14189cad0`, `0x141894d40`, `0x1419734f0`, `0x140d27bb0`, `0x141896700` | `20260717T141700Z-xui20-a33-war-overview-producer-chain-v1-690225df` | `C:\Users\Example\ghidra_agent_exports\20260717T141700Z-xui20-a33-war-overview-producer-chain-v1-690225df\evidence\func_14189cad0.decompile.c`; `func_141894d40.decompile.c`; `func_1419734f0.decompile.c`; `func_140d27bb0.decompile.c`; `func_141896700.decompile.c` | `75b6a514fa19a43a6b2fd5806419b5a22aa86bf620da0c3780ad76df130861b8` |
| war root constructor | `0x141898f30` | `20260717T221351Z-xui20-a33-waroverview-root-constructor-v-8ce15c29` | `C:\Users\Example\ghidra_agent_exports\20260717T221351Z-xui20-a33-waroverview-root-constructor-v-8ce15c29\evidence\func_141898f30.decompile.c` | `1a2904264401e7c082a508a5ef37b360807af2e736237b52b49e4896b84b06e9` |
| W1B metrics and progress | `0x140d29820`, `0x140d23b60`, `0x140d28800`, `0x1406cd5c0`, `0x141973eb0`, `0x141973d70`, `0x141970990`, `0x1419700f0`, `0x1411651d0` | `20260717T221502Z-xui20-a33-waroverview-metric-producers-v-8bd01748` | `C:\Users\Example\ghidra_agent_exports\20260717T221502Z-xui20-a33-waroverview-metric-producers-v-8bd01748\evidence\func_140d29820.decompile.c`; `func_140d23b60.decompile.c`; `func_140d28800.decompile.c`; `func_1406cd5c0.decompile.c`; `func_141973eb0.decompile.c`; `func_141973d70.decompile.c`; `func_141970990.decompile.c`; `func_1419700f0.decompile.c`; `func_1411651d0.decompile.c` | `fa074f8c97dcee14fe88dc1baee44f48737fd018a37b96bc6b819d11f1502d93` |

The extractor interpretation is separately frozen in
`extractor/documents/dossiers/A33_XUI20_EXACT_PRODUCER_LAWS.md`. This replay report
hashes that file only as external authority and never mutates it.
