# Ordinary-map raid hover text

Native text for own active/result raids, incoming raids and target offers in the default map mode. The planning unit icon needs unrecorded client selection and is outside this module.

| Area | Files |
|---|---|
| Identity, hits and worker | [mod.rs](mod.rs), [lane.rs](lane.rs) |
| Recorded joins and diagnostic order | [recorded.rs](recorded.rs), [rules.rs](rules.rs), [diagnostic.rs](diagnostic.rs) |
| Content and naming | [content.rs](content.rs), [names.rs](names.rs), [requirements.rs](requirements.rs) |
| Intel and success | [intel.rs](intel.rs), [success.rs](success.rs), [chance.rs](chance.rs) |
| Trigger-text adapter | [trigger.rs](trigger.rs) |

## Rules

Hit rectangles follow reverse window paint order, not pool/live order; disabled frames own hover. Hits must select their painted target. One coalescing worker binds text to content/frame/perspective, retaining same-icon text across hourly rebuilds and clearing it when icon/country/content changes. Keep joins/localisation off frames. Shared [ToolTip](../../../combat/tooltip_painter.rs) owns resources, wrapping, placement and delayed text.

`rules.rs` owns compact branches, line order and separators. Keep typed string/integer/fixed5 arguments; content chooses precision/colour. Add only missing trailing newlines. Preserve native requirement/pointer-map order, repeated interception occurrences and equipment-group terminal deduplication. Army precedes wing in selected-unit naming.

Project names use hourly owner/definition, not daily history. Captive names use recorded input, not script IDs. Stored wing/ship/state text shares [stored_text](../../../../../dashboard/src/scripted_text.rs); lost localised provenance requires recording/viewer language agreement. Failed reads refuse; older recordings retain usable projections.

Use recorded operands/content for intel, progress, success and sources; carried scores stay carried. Shared evaluators use native raid event scopes and refuse unsupported links per line. Acquire inline assets from all loaded localisation, including runtime-selected text; missing declared sprites refuse layout.

Every refusal names/counts occurrences while independent lines survive. Unreadable raids refuse operand/name lines without hiding other raids. Text failures never change icon eligibility. Distinguish missing, empty, malformed and language-mismatched values; never guess text.

## Checks

[Tests](tests.rs)/[join tests](recorded/tests.rs) cover branches, formatting, names and merges. Byte contracts belong to [engine](../../../../../engine/src/raid_tooltip_operands.rs)/[replay](../../../../../replay/src/extended_rows/raids_projects.rs). Fresh-save comparisons use [raid](../../../../../recorder/src/oracle/projects/raids.rs)/[AirWings](../../../../../recorder/src/oracle/military.rs) oracles; runtime success/intel/order inputs have no save comparator. Follow [viewer](../../../../README.md#compare-changes)/[recorder](../../../../../recorder/README.md) workflows: saved-value agreement is separate from matched hover parity. Addresses/arithmetic stay beside code.
