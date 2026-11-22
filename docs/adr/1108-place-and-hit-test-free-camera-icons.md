# Place and hit-test free-camera icons

A free-camera icon carries a world anchor and its content-defined distance limit. A track, when present, supplies the anchor at the display frame's head. Reject icons behind the camera or outside the draw distance. Keep their authored screen size nearby, shrink towards a bounded minimum, and fade over the last part of their range.

Visibility tests the whole icon along one sight line to its anchor, including terrain and water. Raise the sight-line endpoint and omit the last stretch so the icon does not hide behind its own ground. Widen the fade band with distance to suppress popping. Once an icon is translucent, composite it as a single image. Sort world icons farthest first, retaining their original order on ties and preserving each icon's internal order. Leave unanchored tooltips and combat windows in screen space above the world.

Resolve a hit from the drawn icon's family and key, because content can assign the same priority to distinct families. Transform the pointer back into the icon's unscaled layout, enforce its hit opacity threshold, and give the nearest icon priority. The map camera keeps its own placement path.

[Free-camera placement](../../crates/viewer-native-gui/src/free_icons.rs) · [Visibility and position tests](../../crates/viewer-native-gui/src/free_icons/tests.rs)
