# dashboard-game-window

The game's own windows, opened from a widget for one entity at one replay hour and drawn
read-only: look, scroll and hover, no game actions. A window is the content's `.gui`
template with its sprites, fonts and localisation, resolved through the recording's content
stack (mods included), laid out by the same native GUI code the map viewer draws with, and
bound to the hour's recorded state.

This is a product crate above [dashboard-base](../dashboard-base/README.md). The
[dashboard](../dashboard/README.md) re-exports and registers its two products:

| Product | Answers |
|---|---|
| `game-window` | One window as a [scene](src/scene.rs): its size, every textured quad in draw order with its clip, the list offsets the browser scrolls, the areas the app explains on hover, and the named slots the window binds or leaves unbound. |
| `game-window-artwork` | The scene's texture pages at their native size, as PNG data URLs, by their ids in the window's artwork session. |

## How a window is drawn

- **Templates and resources.** The [session](src/session.rs) reads every `.gui` file of the
  stack in the user's interface viewport (the game's window size over its interface scale,
  from the user folder's display settings, as the map viewer computes it) and loads sprites,
  fonts and textures through `viewer_foundation::map::native_resources::GuiResources`, the
  native GUI loader. Nothing here parses `.gui`, `.gfx` or `.fnt` files of its own. A source
  owns its sessions (one per interface viewport, language and texture decode of its current
  stack): a session holds its stack, so the stack must not hold it, and closing the source
  frees them. Texture page ids are a session's load order, and a request naming a session the
  source no longer holds is refused.
- **Layout.** A window's binder constructs the root and its row windows as
  `native_gui::lifecycle::Instance`s, makes the calls the game's code makes (texts, sprites,
  shows and hides), and [compose.rs](src/compose.rs) lays them out: list rows go where
  `widgets::list_box_rows` places them at the list's scroll offset, the scrollbar after them,
  and every part is drawn in attach order. Refusals and diagnostics keep the native layout's
  counted reasons.
- **Quads.** [scene.rs](src/scene.rs) flattens each laid-out part into the quads its draw
  submits: the same corners, texture coordinates (a sprite's frame offset added) and colours.
  The browser multiplies each page by the quad's colour; it does not execute effect programs,
  so every effect technique drawn that way is listed, counted, in `Scene.effects`. Progress
  bars, fans, masked sprites and flags, animated sprites and baked text sprites are not drawn
  yet: such a part is left out by path and the scene says it is incomplete. Windows draw no
  country flags.
- **Browser.** The UI's `GameWindow` positions the quads absolutely in interface pixels and
  fits the whole window by CSS `zoom`; it never re-flows the content. Scrolling a list asks
  for the window again at the new offset. See the [UI guide](../../ui/src/components/README.md#gamewindow).

## Cost

- **Once per stack**: the session reads every `.gui` and `.gfx` file of the stack, on the
  machine's threads (`ResolvedLayers::read_paths`), and the policies are read against the
  stack's shared content names (`ResolvedLayers::key_names`). When the dashboard opens a
  source whose content is set, `warm_in_background` reads all of this ahead in three stages
  (the source's catalogue, the session, the policies), one step below the priority of work the
  user waits for. Every stage reads the catalogue only over content already bound for its folders (`catalogue::bound_catalogue`), so a read-ahead of content a request has since replaced binds nothing a reply reports, and its session is published only while its stack is still current. One read-ahead runs per source; it holds the source only while a stage runs,
  so once the source is dropped no further stage starts.
- **Names**: a policy's name is the country's stored text, shown only to a viewer in the
  recording's language (`scripted_text::stored_text`); otherwise the row keeps its other
  facts, its name control is hidden and the refusal is counted in the `row name` slot.
- **Once per hour**: the chooser's own four tables are read for the selected hour. Its
  conditions are evaluated on them, and the hour is read again with more tables only when an
  undecided condition's declared inputs read tables it lacks. Each hour's evaluated rows are
  kept per source (the last 32), so scrolling or reopening a window at an hour reads nothing.
- **Per request**: layout and flattening, and the texture pages not yet held, a few at a time.

## Windows

| Target | Root | Binding |
|---|---|---|
| `mioPolicies` | `industrial_organisation_policy_window`, rows `policy_item` in `policy_list_window/policy_list` | [mio.rs](src/mio.rs) |

The chooser's rows are the organisation's recorded allowed policies joined to the country's
policy instances. Each policy's `visible` and `available` conditions are evaluated in the
organisation's scope at the hour by the shared trigger interpreter (reading only the hour's
trigger tables): a policy that is not visible has no row; an unavailable row is drawn with the
app's muted-row treatment (grey and faded as one group, the same as a muted table row), because
the game marks it with its controls' disabled effect, which the browser does not run. A
condition the recording cannot decide withholds the policy by name, or leaves its row unmuted
with the reason on hover; both are counted in the reply's slots.

Root, row and control names are the binding contract. A content stack that replaces or
renames them refuses the window by name; independent facts of the reply remain. Slots whose
game binding is not established are named `unbound` in the reply: their controls keep what
construction leaves, or are hidden where the template's own text is a placeholder.

## Boundary and the deferred extraction

This is the only dashboard crate that depends on `viewer-foundation`
(`tools/check-build-dependencies.py` enforces it); the dashboard process uses its CPU layout,
text and texture decoding only, never a graphics device, a window or an eframe context. Moving
the shared layout into a frame-free crate of its own is deferred. The coupling points that keep
it in the viewer today: `native_gui::Part` and its `Shading` carry the viewer's draw data
(compiled effect assets, fixture techniques and samplers) beside the layout; the same module's
draw functions submit parts to the viewer's sprite `Batch` and `Quad` records; and the native
GUI tests that cover the layout open a GPU device to compare draws, so they would have to be
split before the code can move.

`cargo nextest run -p dashboard-game-window` builds and runs only this crate's tests. They
use synthetic templates, sprites, fonts and recorded frames; tests that read the installation
take it from `BATTLE_PLAN_GAME_DIR`. Nothing opens a graphics device or a window.
