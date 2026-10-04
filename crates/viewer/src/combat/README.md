# Combat windows

Land, naval and result windows use the content stack's native GUI declarations,
retained lifecycle instances and the shared native walk and draw. Sprites, fonts,
texts and bounded lazy portraits resolve through `native_resources`; grids retain
their finalized extents and synchronize their containers' native scrollbars.

Producers bind `Values` by element path and own details, tactics and hover text.
The native geometry supplies clipped hover regions to the shared `ToolTip` in
[tooltip_painter.rs](tooltip_painter.rs). Unsupported content and missing operands
remain named and counted.

Land construction/update: `1415C5900`, `1415C6AB0`, `1415C7520`; naval:
`1417D7450`, `1417D81B0`, `1417DA2F0`. List refill (`1415CA230`, `1415CA460`)
finalizes grids (`1422B2F10`) and synchronizes scrolling (`1422AFA30`).
