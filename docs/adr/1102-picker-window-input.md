# 1102. Keep map picker layout and input with the authored windows

The picker binds modes and toggles supplied by its caller to the retained native GUI and lays them out at the bottom right. The recorded feature selects the three- or four-button layout. Large buttons use slot frames; small buttons use the mode's normal frame or a scripted mode's own sprite. Each slot has selected and deselected controls. An unavailable mode keeps its entry and explains the disabled state. A missing toggle value disables that control. The caller's mode order and configurable boundary govern the available choices; by default the navy slot precedes air.

Use the authored show-all control for the standalone list and right-click a configurable slot to open its configuration and the adjacent assignment list. The list grows upward one authored slot height per scripted mode, holding its bottom edge rather than adding a scrollbar. Show each entry's first shortcut from the slot that currently holds it. The separate find view toggle retains the authored button behavior.

Remember rectangle, paint order and authored alpha hit geometry for each placed control. The topmost target takes the pointer, subject to transparent pixel pass-through. The window background and disabled buttons absorb hits without an action. An open list sits ahead of the picker. A press captured by one window stays with it until release, and a right-click can play the button's authored sound. The output keeps frame click order so callers apply transitions in the same order.

Rebuild the retained windows on content or viewport change. Rebind when the relevant values change. A stationary pointer and unchanged values permit reusing the drawn frame only when no window animates and no button changes or is held. Reuse the picture without replaying spent sounds or clicks.

[Picker](../../crates/viewer-combat/src/picker.rs)
