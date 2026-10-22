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
| `game-window` | One window as a [scene](src/scene.rs): its size, its image as the game draws it, the list offsets the browser scrolls, the areas the app explains on hover, and the named slots the window binds or leaves unbound. |

## How a window is drawn

- **Templates and resources.** The [session](src/session.rs) reads every `.gui` file of the
  stack in the user's interface viewport (the game's window size over its interface scale,
  from the user folder's display settings, as the map viewer computes it) and loads sprites,
  fonts and textures through `viewer_foundation::map::native_resources::GuiResources`, the
  native GUI loader. Nothing here parses `.gui`, `.gfx` or `.fnt` files of its own. It reads
  the stack's presentation, the visual mods the source's widgets show, as the map reads its
  windows. The templates and registry are one choice: the mods', read strictly with what it
  notes kept to the attempt, or, when any of it cannot be read, the recording's whole, named.
  A sprite or font the mods' templates name that the mods cannot draw as they declare it is
  the recording's own declaration of it, read from the recording (the session keeps the
  recording's registry for it); a texture file named by path alone that the mods break is the
  recording's file of that path. Every window the session draws names what it drew that way. Texts show the
  presentation's localisation (`Catalogue::presentation_localisation`). A source owns its
  sessions (one per interface viewport, language and texture decode of its current stack and
  presentation): a session holds its stack, so the stack must not hold it, and closing the source
  frees them. Texture page ids are a session's load order.
- **Layout.** A window's binder constructs the root and its row windows as
  `native_gui::lifecycle::Instance`s and makes the calls the game's code makes (texts, sprites,
  shows and hides). [compose.rs](src/compose.rs) then lays them out:
  - list rows go where `widgets::list_box_rows` places them at the list's scroll offset, with
    the scrollbar after them;
  - grid items go at the slots the code gives them, as the native grid finalizes them;
  - every part is drawn in attach order.
  A binder can explain root controls on hover. Layout needs no graphics device; refusals and
  diagnostics keep the native layout's counted reasons.
- **Painting.** [paint.rs](src/paint.rs) draws the laid-out parts as the map viewer draws its
  windows: each part is emitted through `native_shader::draw::emit` and drawn by the viewer's
  sprite pass into one layer that keeps premultiplied colour, returned as a straight-alpha
  image of the window. Every part draws with its own effect and technique (Up, or Disable for
  a control the code disables), its authored blend and samplers, and its text effects and
  colour runs; the app adds no filter of its own. The crate opens its own GPU device, once per
  process, and each session keeps its texture pages on it. A part the native draw refuses is
  left out by path and reason, and the scene says it is incomplete. Windows draw no country
  flags.
- **Browser.** The UI's `GameWindow` shows the image at the window's interface size, fitted by
  CSS `zoom`, with the hover areas over it; that it is read-only is described to assistive
  technology only. Scrolling a list asks for the window again at the new offset. See the
  [UI guide](../../ui/src/components/README.md#gamewindow).

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
- **Once per hour**: the chooser's own tables are read for the selected hour. Its
  conditions are evaluated on them, and the hour is read again with more tables only when an
  undecided condition's declared inputs read tables it lacks. Each hour's evaluated rows are
  kept per source (the last 32), so scrolling or reopening a window at an hour reads nothing.
- **Per request**: layout, and painting on the session's device (texture pages already
  uploaded are reused until the session loads more).

## Windows

| Target | Root | Binding |
|---|---|---|
| `mioPolicies` | `industrial_organisation_policy_window`, rows `policy_item` in `policy_list_window/policy_list` | [mio.rs](src/mio.rs) |
| `divisionDesigner` | `countrydivisiondesignerview`, cells `designer_division_regiment_slot_entry` / `designer_division_support_slot_entry` in `subunits/*_grid`, rows `designer_stat_entry` in `info/*_stats/stats_grid` | [designer.rs](src/designer.rs) |

The chooser's rows are the organisation's recorded allowed policies joined to its owner's
policy instances. The organisation's own `available` and `visible` conditions and each policy's
`visible` and `available` conditions are evaluated at the hour by the shared trigger interpreter
in the scopes the game gives them (`EventScopes::organisation`): the organisation, whose FROM is
its owner country. A policy
that is not visible has no row; one whose visibility the recording cannot decide is withheld by
name, counted. Each row draws as the game draws it:

- its background takes frame 2 for the attached policy and frame 1 for the others;
- its background and icon draw their Disable technique when the row's action is blocked: by the
  organisation's own conditions, a remaining cooldown (counted in calendar days), or, for an
  unattached policy, its `available` condition or the political power for its attach quote. A
  failing part blocks the row whatever the others are;
- the attach quote is the stored base cost scaled by one factor: the organisation's own
  `military_industrial_organization_policy_cost` entries (its completed traits while it is
  visible and available, its initial trait, its attached policy's organisation modifier) and its
  owner's recorded effective modifier, rounded as the game rounds it
  ([industrial_organisations.rs](../dashboard-base/src/products/industrial_organisations.rs)).
  The owner's political power (exact fixed5) must cover it. A row whose gate the recording does
  not decide (the owner's modifier or political power not recorded, say) has its background and
  icon hidden and counted in the `row state` slot, never drawn in a guessed state;
- an unattached row with a positive quote shows it through the content's
  `INDUSTRIAL_ORG_POLICY_PP_COST`; the attached policy and a quote of zero or less hide the cost,
  as the game does, and an unknown quote hides it, counted in the `row attach cost` slot; the AI
  value is hidden, as the game hides it with its AI view off;
- the icon is the policy's own, else the sprite for its country and token, its token, then the
  unknown idea's sprite; a name that is not declared is refused and its control hidden.

The chooser's tables are sampled daily; a reply names the hour its rows were read.

The designer is the template's roster owner's own. Composition cells come from the recorded
spatial layout, and each is attached in the state the designer's refresh gives it
([slots.rs](src/designer/slots.rs)): filled, addable, locked or blocked, from its column's
count of real definitions, the column's usable height and whether the template is editable.
Holes keep their place.
- A filled cell shows its definition's `GFX_unit_<name>_icon_medium` glyph, in the NATO frame
  when the viewer's game shows NATO symbols. An undeclared glyph is hidden and named, never
  replaced by another unit.
- The country's brigade-height unlock is not recorded, so the regiment grid's usable height is
  every value the defines allow. A cell whose state depends on that height is left out and
  counted.
- The header shows:
  - the template's name;
  - its symbol: the custom `GFX_div_templ_<n>_large` symbol, else the priority unit's glyph;
  - the priority frames from the defines' stage count;
  - the paradrop frame;
  - the equipment button's sprite;
  - the country's Army experience.
- The stat rows and summary values are the designer stats of
  [dashboard-army](../dashboard-army/README.md#designer-stats). Rows keep the game's fixed
  order. A value not yet proven shows its label only, with why on hover.
- Unbound, each named in the reply's slots: the special-forces frame, the equipment warning,
  the production footer and an army headquarters' composition. Their controls are hidden.
- Pickers and overlays stay closed. The editor's buttons draw as the game draws them over a
  template with no pending edit: reset and save disabled. Duplicate, whose enabling depends on
  the designer's mode, which is not recorded, is hidden and named. The app says on hover that
  they do not act. The equipment button is enabled only while the regiment grid holds a real
  definition, as the game's is.
- Everything the game draws resolves through the recording's content stack, with no vanilla
  fallback: the `.gui` layout, frames, backgrounds and sprites, unit glyphs and the symbol,
  fonts and their text colours, and localisation. A mod that restyles the designer, replaces
  a sprite's texture or a font's glyphs, or adds a sub-unit draws in its own style; what the
  renderer cannot reproduce is refused by name and counted, never drawn in another style.

Root, row and control names are the binding contract. A content stack that replaces or
renames them refuses the window by name; independent facts of the reply remain. Slots whose
game binding is not established are named `unbound` in the reply: their controls keep what
construction leaves, or are hidden where the template's own text is a placeholder.

## Boundary and the deferred extraction

This is the only dashboard crate that depends on `viewer-foundation`
(`tools/check-build-dependencies.py` enforces it); the dashboard process uses its layout, text,
texture decoding and sprite pass, on a GPU device of its own, never a window or an eframe
context. Moving
the shared layout into a frame-free crate of its own is deferred. The coupling points that keep
it in the viewer today: `native_gui::Part` and its `Shading` carry the viewer's draw data
(compiled effect assets, fixture techniques and samplers) beside the layout; the same module's
draw functions submit parts to the viewer's sprite `Batch` and `Quad` records; and the native
GUI tests that cover the layout open a GPU device to compare draws, so they would have to be
split before the code can move.

`cargo nextest run -p dashboard-game-window` builds and runs only this crate's tests. They
use synthetic templates, sprites, fonts and recorded frames; tests that read the installation
take it from `BATTLE_PLAN_GAME_DIR`. Tests that paint need DXC and a GPU and are ignored; run them
with `--run-ignored all` and `BATTLE_PLAN_SHADER_COMPILER` set.
