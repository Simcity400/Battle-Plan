# Combat windows

Land, naval and result windows bind recorded values into native GUI declarations. The shared walk/lifecycle/compiler owns layout and drawing; producers own combat details, tactics and hover text.

| Area | Entry points |
|---|---|
| Content, binding, presentation | [assets.rs](assets.rs), [binding.rs](binding.rs), [mod.rs](mod.rs) |
| Recorded calculations | [land_details.rs](land_details.rs), [naval_details.rs](naval_details.rs), [keyed.rs](keyed.rs) |
| Tactics and text | [tactics.rs](tactics.rs), [modifier_text.rs](modifier_text.rs), [tooltips.rs](tooltips.rs) |
| Marker hover and history | [land_marker_tooltips.rs](land_marker_tooltips.rs), [naval_marker_tooltips.rs](naval_marker_tooltips.rs), [icon_frames.rs](icon_frames.rs) |

## Rules

- Bind `Values` by path through native resources, including bounded lazy portraits. Preserve membership order, finalized grid extents and scrollbar synchronization.
- *Match game* rasterises before display stretch. Keep active/reserve name truncation and font-measured three-dot ellipsis.
- Hovers use clipped geometry, reverse attach order and [ToolTip](tooltip_painter.rs); transparent text passes through. Shortcuts append native keycaps/delay. Ship groups anchor beside the window; sunk entries follow grid slots. Naval markers click only backgrounds/visible HQ frames; balance icons hover.
- Land marker counts include reserves; countries keep primary-list order. [Border tails](border_marker.rs) use recorded state/event operands and shared effect descriptions; missing/unsupported fragments refuse by name/count.
- Naval positioning, screening, carrier air and anti-air refresh from runtime operands rather than lagging combat copies. Index joins once per refresh; missing/duplicate keys and incompatible fixed modifier definitions leave values unknown.
- Commander roles use recorded category/skill; terrain hovers use content adjusters/widths. Timed-trait gaps refuse only affected hovers. Exile flag/frame withhold together on missing inputs. Naval trait context differs from land; only naval shows field-marshal inactivity.
- Retained naval results classify recorded ships and aircraft using native country equivalence, representatives and survivor priorities. Stored result text keeps the recording-language guard; both panels open locally and scroll through retained text bars.
- Scripted tactic text follows the viewer's NATO choice, including nested unit icons; toggling refreshes land bindings without changing a disengaged naval window's supplying hour.
- Unseen icons prefer established view/index frames, else land's authored/naval binding initial frame with a counted history reason. Fallback never establishes history.

The [viewer guide](../../README.md#native-gui-and-hover-ownership) owns lifecycle, clocks, language guards and matched comparisons. Use [AGENTS.md](../../../../AGENTS.md) checks.
