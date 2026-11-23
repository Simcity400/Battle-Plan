# 1110. Reuse admitted map windows without losing their native state

An icon's owner keeps its pool slot until the update pass erases it. Requests use a round-robin cursor. Exhaustion answers without a slot and schedules growth on a later tick. The pool can exhaust on off-screen requests before the admission pass removes them. Released slots retain their window control state for reuse.

The pass updates the windows live at its start. Outside or empty icons are erased, hide their child lists through the native lifecycle, and release ownership. A swap removal does not independently cause the replacement item to run twice. A released window keeps ticking a pending fade so its eventual child hide still reaches the list. Its slot is outside the visible list while that happens. A live admitted icon can remain visible through its own fade and leaves the visible list at the end.

Refusals belong to the window instance while its inputs are unknown. A write clears only the named part it establishes. An update that establishes all unknown calls can clear their refusal. An inapplicable call that requires rebuilding the instance retains its latest reason. An occupied admitted window retains its list position and named refusal, and a released slot carries the same unknown state to its next owner until the required update occurs.

[Window pool fixtures](../../crates/viewer-native-gui/src/native_gui/tests.rs)
