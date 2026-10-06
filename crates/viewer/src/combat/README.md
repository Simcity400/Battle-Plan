# Combat windows

Land, naval and result windows use the content stack's native GUI declarations,
retained lifecycle instances and the shared native walk and draw. Sprites, fonts,
texts and bounded lazy portraits resolve through `native_resources`; grids retain
their finalized extents and synchronize their containers' native scrollbars.

Interactive Match game rasterises this GUI at the game's framebuffer scale,
then linearly presents the completed image at the display stretch (`142224600`,
`1422206C0`). This preserves the initiative overlay's opaque one-texel border
and thin font strokes. Active and reserve names both retain the native
`truncate` flag through binding, preparation and retained refresh; ellipsis is
three font-measured dots (`1422D7D10`, `142286770`, `14228DE40`, `14228B2F0`).

Producers bind `Values` by element path and own details, tactics and hover text.
The native geometry supplies clipped hover regions to the shared `ToolTip` in
[tooltip_painter.rs](tooltip_painter.rs). Unsupported content and missing operands
remain named and counted.

Land construction/update: `1415C5900`, `1415C6AB0`, `1415C7520`; naval:
`1417D7450`, `1417D81B0`, `1417DA2F0`. List refill (`1415CA230`, `1415CA460`)
finalizes grids (`1422B2F10`) and synchronizes scrolling (`1422AFA30`).

Commander portrait headings use the recorded leader category and skill to select
the native localised role before the name (`140C0A7A0`, `140C0A9A0`). The land
terrain banner's hover reads common adjusters, widths, modifiers, supply-flow
reduction and movement cost from the recorded content stack (`14142A4C0`).

Map land-combat hovers (`land_marker_tooltips.rs`, `1418AC1D0`) use the shared
engaged index. Counts include reserves; country names follow the recorded primary
list's order. Outlook uses survival scores, with primary or secondary country
membership making the wording relative to the viewer (`1413CCBC0`). Unknown
engagement, naming operands or score operands produce counted refusals. Border
conflict effect tails need unrecorded combatant country, state and event operands
(`1413D69D0`) and are refused while the common lines remain available.

Naval marker hovers (`naval_marker_tooltips.rs`, `1418E2DC0`) add task forces,
convoys, ordered countries and the viewer's first qualifying naval HQ count.
Only the background and visible HQ frame buttons receive clicks (`1418E42A0`);
the balance icons receive hovers. Ship group hovers anchor beside the combat
window (`1417D47E0`); sunk entries follow the authored grid slots (`1417D05D0`).
Carrier grouping shares the equipment-pool predicate used by sortie modifiers
(`14195ED00`, `140C2B7E0`), and runtime air sprites load from the stacked registry.

Combat hovers reject transparent text and follow graphical attach order
(`1422D8240`, `142226BD0`). Authored button shortcuts append native keycaps and
use the normal show delay (`1402A2810`, `1402A4080`). Naval header skills use
recorded component values and retained definitions (`141DF4270`, `140AD9010`);
trait hovers are refused by name and counted until timed traits are recorded.
