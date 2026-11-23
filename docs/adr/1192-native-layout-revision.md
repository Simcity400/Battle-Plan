# 1192. Cache a window layout at its own origin

A native GUI instance's layout is at its own origin. Moving the window changes placement without changing the layout revision, its transition history or its animation state. A tick is idle when the window has ticked, no call other than a move has written it since, and no program changes on its own. An idle tick can stamp time without changing a settled draw. A non-idle tick or a write makes the revision advance.

`Instance::layout_kept` retains the most recent pointer-free layout by instance revision, root identity, the caller's resource stamp and whether hit regions were requested. The instance compares the root address for identity. The caller changes the stamp when asset, template, country or viewport state changes the draw. A different root, resource stamp, region mode, write or active tick can therefore invalidate the layout while a move alone cannot. Kept pool windows can use the full tick's layout without recomputing it on a following idle frame.

`Shading::as_drawn` disregards a settled part's clock for drawing identity because the same program gives it the same output at each clock once settled. Active draw programs still use their own clocks.

[Layout cache](../../crates/viewer-native-gui/src/native_gui/lifecycle.rs) · [Drawn shading](../../crates/viewer-native-gui/src/native_gui.rs) · [Window pool consumer](../../crates/viewer-native-gui/src/icon_windows.rs)
