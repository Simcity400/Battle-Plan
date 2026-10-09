# Campaign map

## What it shows

The native viewer’s map inside a widget body; React owns its rectangle and status text.

## Products it reads

None. [geometry.ts](../../dashboard/geometry.ts) coalesces map bodies and tracks displayed source/branch/hour/sequence acknowledgements. Rendering belongs to the [viewer](../../../../crates/viewer/README.md).

## Settings

Country and campaign/follow/pin. The shared map pop-down supplies appearance, branch and export controls, nesting the common widget settings.

## Notable behaviour

- A queued target is not a displayed frame: Updating lasts until acknowledgement. During Follow playback, the latest built hour at/before the clock can display with its own date. Paused, pinned and exported targets stay exact.
- Following targets carry seekSequence independently of hour sequence, preserving train/weather animation across skips and resetting on seeks. Pinned hour changes start new history.
- Outside-branch hours show no neighbouring map. Keep map pixels and fidelity logic in the native viewer.

## Files

| File | Responsibility |
|---|---|
| [MapWidget.tsx](MapWidget.tsx) | Body registration and native status. |
| [MapWidgetControls.tsx](MapWidgetControls.tsx) | Map/header controls. |
| [presentation.ts](presentation.ts) | Target/acknowledgement matching; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts) | Metadata and registration. |
