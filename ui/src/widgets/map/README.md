# Campaign map

The native map drawn into a widget body. React owns the rectangle and the status text; the viewer process renders the pixels for the target hour, country and fog setting.

**Data:** no product. The widget registers its body with the shared map geometry store (`dashboard/geometry.ts`), which sends one coalesced layout of every map body to the viewer and reports back the displayed source, branch, hour and sequence.

**Header controls:** `MapWidgetControls` replaces the settings gear with the shared map pop-down (appearance, branch, export) and nests the common widget settings inside it.

**Rules:** a queued target is not a displayed frame; the status shows "Updating" until the acknowledgement answers the request (`presentation.ts`). While a following map plays, the native map may answer with the newest hour built at or before the clock: its date shows, without an updating label, and it returns to exact hours when the builders catch up. Paused, pinned and exported maps stay exact; the target's `playing` flag says which. Out-of-branch hours show no map rather than a neighbouring one.

**Remaining:** none open in the widget. Renderer fidelity is tracked in `crates/viewer/README.md`.
