# Reuse map icon window layouts across frames

Run the native pool manager for admission, writes and erasure even when an icon family can reuse what it drew. An owner admitted within the viewport pad can move into view between layout passes. Off-screen dirty provinces can retain a window until the rotating cleanup phase visits them. A manager-only pass advances changed windows without laying them out. A subsequent drawn frame lays out any changed window, and visible changes or pending requests can request that frame.

A kept frame reuses settled layouts at new integer origins when only the camera or owner moved. Rebuild a layout when ownership changes, its inputs change, it animates on its own, or a failed frame, changed viewport or grown pool invalidates its history. Compare recorded owner inputs only when they are the update's sole dependency. Keep the last layout time for settled content. Lay out a frame without a clock only when suitable history exists. Reject a frozen dump's new allocation demand.

Skip the manager pass when its owners, viewport and inputs are unchanged, no dirty-window cleanup is due, all visible layouts are complete, and no request or animation needs work. The free slot a future owner receives does not affect its draw. Preserve generation numbers across pool rebuilds. Reuse the same compiled quad allocations for a retained layout and anchor, update their layer rank and shift them to the new origin. Drop paint records for undrawn owners, and batch diagnostics per presentation. Continuity tests subtract movement of the owner's projection before measuring unexpected layout movement.

Use one content-derived GUI viewport for parentless layout, projection and reading. Unsupported or undisclosed hit shapes block input within their reported bounds and never dispatch callbacks. For a supported observer, use the native geometry when available. A zero stretch alone cannot show that a native rectangular hit is empty.

[Icon windows](../../crates/viewer-native-gui/src/icon_windows.rs) · [Viewport and hit geometry](../../crates/viewer-native-gui/src/gui_resources.rs)
