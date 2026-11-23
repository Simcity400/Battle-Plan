# Preserve element-specific GUI field and construction rules

GUI field readers share the token stream but retain each element's constructor defaults, supported child kinds and rules for replacing data. A common schema that fills a value whenever it encounters a matching name would change window placement, drawable construction and lookup. Read each field through its element's branch and refuse fields whose native result cannot be determined.

## Placement and animation

Container position assignments also set both animation endpoints. The show position sets the reference for position-relative sizes. `step` is retained when authored even though the position and size evaluators do not use it. The constructor's unset clipping state enables clipping when there is a background. A malformed explicit assignment has a different result because the reader clears the local before writing it. Container and window animation types have distinct native mappings. Shared animation assignments write both directions. Fade delays, positioning and background drag behavior use each field's own stored values. Scrollbars initialize visible only under the matching autohide setting. A peek key marks the hover behavior as authored even when its operand does not scan successfully.

Container and grid orientations use the nine-keyword domain, while the text orientation mapping has seven values. Grid numeric and keyword handling differs from its string fallback. Some text formats recognize `centre` alone, while other controls accept `center` too. An icon stores the results of its integer point reader as floats, and consumers use the native float-to-integer conversion. Loss of precision at large input values is observable. An authored spelling handled as a numeric token cannot dispatch into the icon's named coordinate fields.

## Controls and text

Several sprite keys can write the same stored sprite. Buttons may receive their sprite from inheritance and have no parsed orientation if they are outside the inheritance traversal. Construction only projects a positive frame. Preserve the distinct icon and button transparency aliases, sound alias ordering and explicit suppression of click defaults. Checkbox uncheck sounds fall back through the button's resolved click sound. An element's hidden bit also affects later show operations.

An instant text box has a default construction width when its stored width is zero. Its scrollbar changes text clipping. Text sprites keep accepted blocks in order and compare their format keyword using the text-sprite spelling rules. The bound fields and map text blocks use the shared value readers. An element which draws no text from a stored block does not need an unavailable point operand merely to omit that drawing.

Grid fields include the first background and preserve the odd consequences of an extra component writing the next stored format, orientation or fill field. A list box can retain uninitialized priority separately from a numerical assignment. Its background is a name, and its horizontal layout, scroll side and spacing are interpreted by the list-box reader. A drop-down keeps its own expand controls and an inner container that owns delegated common fields and children. Named fields such as the expand-frame switch retain the distinction between unset and false.

## Words shared by several fields

For the point components whose native storage continues into the next field, keep the modeled words in element order. Edit-box position, size and border components can reach the later words used for cursor and flags. Browser and curve-graph components can reach size and then the sprite string. Scrollbar components can reach its bounds, step, start value, priority and the field list examined during inheritance. Some writes touch unused padding. Refuse a later consumer that would depend on an unavailable object reference or field instead of manufacturing a value for it. The unset minimum, field-list count and distinct child registry matter even for scrollbars without a parent.

[Typed GUI elements](../../crates/content-reader-gui/src/gui/native.rs) · [Element readers](../../crates/content-reader-gui/src/gui/native/elements.rs)
