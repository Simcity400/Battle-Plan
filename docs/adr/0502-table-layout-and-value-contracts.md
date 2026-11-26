# Keep table placement stable as rows and values change

The browser scroll range is capped below Chromium's layout limit. The native scrollbar still covers the entire logical data set. Around the viewport, rows retain their layout sizes and keyboard targets stay reachable. An oversized focused row anchors at its start so repeated key presses do not oscillate around a sticky header. Windowed rows retain the keyboard target even outside overscan.

Wrapped-row measurements use a baseline for the current typography and width and retain exceptions above that floor. Treat half-pixel differences as equal and reuse the measurement object when it would place every row the same way. This avoids committing on each mount when every compact row measures a pixel taller than the CSS minimum. Measurements from another scope remain useful until their rows are observed again.

Sort on exact values and break ties by stable row key in either direction. Approximate numeric keys can order distinct integers cheaply because Number conversion is monotone. Values with matching approximate keys compare exactly. Missing values are handled separately and can gate custom content. Dated group keys supply identity, so equal display labels remain distinct. The ordered tree keeps the first place a row key appears and drops later repeats, including cycles.

Live cell text, sort order and bars can share the same head value. Structural bars and bars fed by a second live cell can move independently of a painted label. Content revisions identify geometry changes without requiring different label text. A narrow table may hide columns by priority, with later columns hiding first on ties. It remembers each removed width to decide when a column can return. Hidden values follow the widget's own tooltip rows, with the shared scroll surface keeping expanded tooltips reachable. Widgets that prefer all values can opt into horizontal scrolling.

[Logical scroll geometry](../../ui/src/families/host/scroll.ts) · [Table model](../../ui/src/families/table/model.ts) · [Table layout](../../ui/src/families/table/layout.ts) · [Table options](../../ui/src/charts/table/options.ts)
