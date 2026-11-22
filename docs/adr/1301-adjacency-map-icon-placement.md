# 1301. Use content-defined rules and camera distance for adjacency icons

An adjacency rule that names an icon province supplies the icon for that province. Later rules can replace its link. The viewer positions the declared icon window at the province anchor plus the authored offset, aligned by its left edge and vertically centered. Every quad in the window shares this anchor for free-camera placement. Further offsets are screen-space adjustments.

The icons obey the active mode's adjacency admission and its content-defined priority. The stock camera measures the cutoff from its height. A free camera measures distance to each icon. The comparison is strict, the draw walks farthest first, and an unreadable cutoff withholds the icons with its reason. The rule's current trade or passage status does not affect admission.

[Adjacency icon preparation and tests](../../crates/viewer-map-icons/src/adjacency_icons.rs)
