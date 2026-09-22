# Campaign map

The native map drawn into a widget body. React owns the rectangle and the status text; the viewer process renders the pixels for the target hour, country and fog setting.

**Data:** no product. The widget registers its body with the shared map geometry store (`dashboard/geometry.ts`), which sends one coalesced layout of every map body to the viewer and reports back the displayed source, branch, hour and sequence.

**Header controls:** `MapWidgetControls` replaces the settings gear with the shared map pop-down (appearance, branch, export) and nests the common widget settings inside it.

**Rules:** a queued target is not a displayed frame; the status shows "Updating" until the acknowledgement matches the request. Out-of-branch hours show no map rather than a neighbouring one.

**Remaining:** none open in the widget. Renderer fidelity is tracked in `crates/viewer/README.md`.
