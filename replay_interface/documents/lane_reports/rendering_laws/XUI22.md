# XUI22 - generic scripted-GUI state

**Status: GENERIC FAIL-CLOSED CONSUMER IMPLEMENTED; D84 PRODUCER NOT YET PROMOTABLE**

All 23 inventoried national-mechanic definition files are admitted, with zero parser-omitted windows. The production implementation is one effective-VFS registry and one variable/flag predicate/effect evaluator; it contains zero identifiers for the 23 panels and therefore admits mod-added scripted GUIs through the same path.

`ScriptedGuiView` now consumes the actual d51 production carriers (`extra._r2_script_variables`, `extra.script_flags`) while retaining the legacy root-variable path only when the production carrier is absent. It also exposes the exact `scripted_gui_runtime` carrier. Supported variable/flag predicates evaluate to concrete tri-state results and supported effects execute only as an atomic read-only preview. Concrete local visibility, enabled, and property results remain authoritative; A38 is consulted only on `Unavailable`. Definition-level enabled state participates in every effect gate, malformed named/property blocks reject production admission, missing scopes remain unavailable, division by zero rejects, and arbitrary engine effects never mutate `WorldFrame`.

Variables and flags do not determine arbitrary engine triggers, scopes, or `[GET_X]` dynamic properties. That claim is explicitly refuted. A38 now has a frozen current-build manager/updater/widget layout, a bounded SHA-gated static CIcon proof, and a generic read-only live probe. Wave-5 Ghidra closure proves the `+0xA50` rows cache resolved element/property names rather than final values; exact text, image/frame, and layout outputs are consumed immediately by widget setters and require synchronized owner/setter observations. Exact values still require a running process because retained RenderDoc frames contain GPU state rather than these CPU-side materializations. Replay remains fail closed until each required value is locally evaluated or captured.

The replay trust boundary no longer accepts the old five-field runtime row as an unauthenticated legacy downgrade. Every `extra.scripted_gui_runtime` payload must use the exact d84 row shape and carry the full `_r2_scripted_gui_runtime_live` authority envelope. Removing both the authority and the row identity fields now poisons the complete runtime surface instead of bypassing d84 authentication.

Mechanical census: 23/23 panel files admitted; 0 parser-omitted windows; 40 effective scripted-GUI definitions; 182 named triggers; 133 effects; 31 dynamic calls; 0 bespoke panel identifiers in the evaluator.

Cleanup: 51 historical XUI22 test roots removed (23,615 logical bytes); zero matching replay-lane temp/cache paths remain. Tests now remove their temporary VFS roots after registry loading.

Focused gates: game_ui evaluator 3/3; replay_core d51/runtime roundtrip 1/1; game_ui compile PASS. Full libraries: game_ui 120 passed, 0 failed, 1 ignored external installed-game check; replay_core 45 passed, 0 failed. Verification: 25/25 PASS.

## Binary provenance

| Function address | Job ID | Artifact path | Manifest hash | Disposition |
|---|---|---|---|---|
| `0x140B76400`, `0x140B79C90`, `0x140B79820` | `20260717T103021Z-a38-registry-loader-v1-d393bd4a` | `C:\Users\Example\ghidra_agent_exports\20260717T103021Z-a38-registry-loader-v1-d393bd4a\evidence` | `227d64f3bd062fad805431cd1b488e03dd0bcc5b860e3088266f5f5cf724cf6d` | Manager construction, updater array, update lifecycle. |
| `0x140B77940`, `0x140B78760`, `0x140B7BBF0`, `0x1405354A0`, `0x14229A780` | `20260717T103523Z-a38-gui-field-evaluators-v1-1cb25957` | `C:\Users\Example\ghidra_agent_exports\20260717T103523Z-a38-gui-field-evaluators-v1-1cb25957\evidence` | `ec800e13fcd8a8679cf23eade0d590561729090d4dc889515becb58bfdc81935` | Binder, evaluator, context join, image application. |
| `0x14228F080`, `0x14229ABE0`, `0x140CF9200`, `0x1422E6410`, `0x1422E6490`, `0x1422E6460`, `0x1422E6A90` | `A38-CICON-STATIC-870EBF-V1` | `replay_interface/re_evidence/rendering_laws/XUI22/a38_cicon_static_probe.json` | `54a7bab9ec81c7ea13f2191212e1a841ad0148afcbf296c9216dda1d00aba978` | Exact-SHA RTTI/vtable/accessor bytes. |
| N/A | `XUI22-FREEZE` | `replay_interface/re_evidence/rendering_laws/XUI22/manifest.json` | `f6507c0781102a8dc34183f0cb9aa4fa3fe29823b9ba95711c37f726aaf3bac8` | Composed source/evidence freeze. |

