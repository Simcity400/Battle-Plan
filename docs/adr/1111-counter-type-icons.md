# Resolve counter icons through the authored sprite registry

The GUI's type child supplies placement, and the counter selects the sprite independently. Use the recorded priority sub-unit name verbatim rather than the model sprite field. Ordinary sprites provide pictorial and NATO frames, with a one-frame strip supporting both through its only frame. Hidden counters choose the no-intel sprite. Custom division icon indices follow the game's signed sixteen-bit narrowing and the applicable NATO option rule. A negative narrowed index gives no custom icon. The generated sprite name is canonical decimal and zero-based. The small counter-sized icon has a separate variant from the designer version.

A declaration's presence controls custom selection, even when the corresponding resource was refused. An undeclared icon falls back to the sub-unit frame. A declared but refused icon is withheld with its reason, since replacing it would show a different selection. Read the latest declarations from the mod stack to include newly authored names without maintaining a built-in icon list.

Pack the selected icon families onto one atlas page to avoid texture switches between counters. Decode block-compressed files before packing and use loaded, block-rounded extents and the authored frame direction for the grid. Hit geometry uses the original texture size and frame grid independently of integer frame extents. A missing or broken visual replacement can be drawn from the recording with a named fallback.

[Template icon selection and atlas](../../crates/viewer-foundation/src/templates.rs)
