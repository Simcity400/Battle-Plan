# 1102. Bind the minimap to recorded content and viewer input

The viewer hosts the minimap with the retained native GUI used by the picker. The world picture is static. Mouse press and drag set the camera target, and wheel zoom is centered at that target. A pointer across either end of the picture wraps horizontally. Its north and south values clamp to the map, with north at the top of the picture.

The outline describes the camera's ground footprint in world coordinates. Its input points are top left, top right, bottom left, bottom right, and middle. The world picture rectangle is in interface pixels. Four lines outline the footprint, with seam copies for the far vertical line and the two horizontal lines. The picture's container clips every line, and the two-pixel line sprites take half the requested length as stretch. A footprint with depth less than one tenth of a world unit, or an absent footprint, leaves the previous outline in place. Coordinates round ties away from zero.

The handle's first authored shortcut also toggles the minimap. Duration and shift follow the matching authored defines. Reversing a slide mirrors its progress through the easing curve to hold the current position. Retained geometry rebuilds when content or viewport changes, and the authored tooltip anchor is 90 pixels above the hovered control's top-left corner. Failed localisation leaves that tooltip unset.

The offensive and defensive controls arm a ping, as do Ctrl and Alt held together. Shift makes a held-key or armed defensive ping offensive. The recording has no ping stream and the replay never sends the game command. The pinged handle and circle therefore stay hidden, with the refusal named in the window.

[Minimap bindings](../../crates/viewer-combat/src/minimap.rs)
