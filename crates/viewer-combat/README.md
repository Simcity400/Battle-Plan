# Viewer combat

Land and naval combat windows, retained result windows, bindings, calculations and hover text. Content contains the immutable operands each window reads; the viewer host builds a fresh content Arc with every verified content load and supplies the current frame, playhead and shared engagement history.

Native GUI layout, flags, textures, text, visibility and engagement history come from `viewer-foundation`. Combat refresh keeps its demand gates, retained naval contents, list membership order and presentation clocks. Source details are in [the module guide](src/README.md).

`test-support` exposes existing binding and GUI contracts for the viewer host's composition tests; shared synthetic inputs come from foundation's opt-in test support.

Recorded source-label recovery is shared with occupation hovers through viewer-foundation's `recorded_labels`; combat re-exports the shared reader.
Division combat summary hovers include stored experience normalized by recorded manpower. Missing experience operands retain a named refusal.

Division combat summary hovers show career entries and honors from recorded history, localizing event and medal identities through the recorded content stack. Event subjects retain the recording language's stored-text semantics; missing operands and unsupported event kinds are named and counted. Province captions share the existing recorded rename, qualified victory-point, state and region fallback chain.

Division summaries show pending conversion using the recorded old and current template names.

Division summary hovers include deployed active-cache statistics, the retained career counter and officer XP. Award effects resolve their definitions from recorded content, preserving instance order and diminishing repeated-award scales; on-grant effects use the shared unit-scope description engine.

Owned single-division map hovers expose the same experience, pending conversion, deployed values and career as combat summaries. Content/hour/base-hover identities retain the composed tooltip. Anti-air reuses the existing raw movement operand. Width uses effective template, country and selected commander operands; speed uses current movement, fuel, scale and tactic context. Shared certified country and selected commander roots supply modifier explanations, filtered to the deployed army domain and composed through recorded modifier metadata and source labels. Fresh attrition derives each nonzero held variant occurrence once, selected terrain, supply, state/country context, weather, acclimatization, training and border power. Individual missing certificates or labels retain named refusals; available source fragments remain visible.

Deployed cache scalars join DivisionPanelStats by division; independent progression and live movement operands remain in DivisionStatus. Missing panel operands retain their individual refusals.

Naval source-label recovery is shared through `viewer-foundation::naval_labels`; the combat module reexports that same API. The viewer prepares one recording-language index and shares it with Navy mission controls.
