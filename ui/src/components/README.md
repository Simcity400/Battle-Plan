# Shared components

Reusable controls, popovers, tooltips, country choices and flags. Shared appearance
and control conventions live in the [frontend guide](../../README.md#shared-appearance-and-controls).

## AircraftPicker

[AircraftPicker.tsx](AircraftPicker.tsx) chooses Fly mode's aircraft from the recording's [aircraft catalogue](../../../crates/dashboard/README.md#aircraft-catalogue): any aircraft model or entity of its content stack, or none. There is no default: until one is chosen the trigger reads "No aircraft chosen", the list says Fly mode is a free camera, and the list opens by itself when Fly mode starts with nothing chosen. The list docks in the shell's side column beside the workspace, which narrows the native map rather than covering it; it is not a popover, so the map-overlay guard leaves the map shown while planes are tried. Its rows page the catalogue through [aircraftCatalogue.ts](../dashboard/aircraftCatalogue.ts), asking only for the pages of rows it shows, with a search over names, titles and references and a scope of aircraft or every registered entity and mesh. Refused models stay listed, greyed, with their reason, keep their places but are no keyboard destinations, and are counted by kind. Choosing one sends it to the map and keeps the list open: the map shows the chosen plane, which is its preview, and the status line follows what the map answered (loading, flying, or why it cannot be flown). Escape or the close button closes the list. [aircraftChoice.ts](../dashboard/aircraftChoice.ts) keeps the choice per recording on this device.

## GameWindow

[GameWindow.tsx](GameWindow.tsx) shows one game window (the `game-window` product) as the game draws it at the hour shown: the service lays it out in the window's own interface pixels and paints it through the native GUI draw, and the browser shows that image as it is, with no filter, tint or mute of its own. The layout is Rust's: the window is fitted whole into the space it has by CSS zoom, never re-flowed; below its smallest legible zoom (`MIN_ZOOM`, 0.6) it scrolls in its space from its start instead of shrinking further or clipping; and scrolling a list asks for the window again at the new offset (once the wheel settles), showing the previous one until it arrives. A window belongs to its client, content folders, recording and target: a change of any asks for it afresh and hides what was drawn. Rows are focusable list items named with every detail their tooltip shows; Tab reaches the window and its rows, and arrows, Page Up/Down, Home and End scroll the focused row's list along its own axis (right and left for a list that scrolls across). A row focused outside its list's box has the window laid out again at the offset that reveals it; clips never scroll on their own. Hover areas explain themselves through the shared tooltip; codes appear only there. Nothing is drawn over or beside the image: that the window is read-only is a visually hidden description its scene names (`aria-describedby`), and the focus outline is the app's own. A reply's image is decoded before it replaces the one shown, so hover areas never sit over the previous drawing. Refusals, notes and unfilled slots go to `onDiagnostics`, never on screen. [GameWindowPopover.tsx](GameWindowPopover.tsx) is the drop-in trigger: a button that opens the window in a popover headed by its title. The renderer is lazy-loaded, and nothing is asked for while the popover is closed.

## VirtualList

[VirtualList.tsx](VirtualList.tsx) renders measured, variable-height ordered rows
inside an existing scroll surface. It binds to the committed `scrollRef` element,
preserves authored root padding, row gaps and fractional layout heights, and mounts the viewport
with overscan plus a focused row and, when needed, one entry row. Every mounted row
carries its complete logical `aria-posinset` and `aria-setsize`; the list label is optional.
The optional root `style` preserves consumer presentation; padding belongs to
the measured list extent, row positions and keyboard scroll destinations.
`flowWhenFits` keeps the same ordered-list and row elements in native flow when
the complete measured extent fits the viewport, and windows overflowing rows.
Switching layout retains their keys, focus and keyboard state.
Measurements and retained details expansion belong to `identity`. Whole-row
remounts restore expansion; conditional descendants destroyed inside a mounted
row start fresh when recreated. Descendants that remain mounted keep their native
state. Consumers that control their own details use `retainExpansion={false}`.

`itemKey` identifies both retained focus and pending keyboard handoffs. Reordering,
inserting or removing other rows reconciles their positions in the same render,
so a surviving focused control remains mounted even outside the viewport.

Rows are interactive by default. `isInteractive` identifies rows with native
sequential controls, including arbitrary static prefixes, tails and internal runs.
Static rows retain their ordered-list membership. Replace the immutable `items` array or
the predicate whenever eligibility changes, and keep the predicate stable when
its rules are unchanged. Eligible positions are indexed once per such revision;
keyboard destination lookup is logarithmic and never calls the predicate.

Tab and Shift+Tab retain native order within a row, seek another logical row only
at a control edge, and leave natively at the first or last eligible control. Native
external entry reaches the first or last logical control across adjacent lists.
An entirely static mounted window keeps one real eligible row mounted for entry;
a fully static logical list retains native scroller behavior with no added tab
stops. Internal focus handoffs remain internal even when scrolling unmounts the
origin. Backward handoffs wait for retained details and conditional descendants.
Home, End and the vertical arrows also use eligible logical destinations.
Tab boundaries and destinations exclude negative tabindex, disabled controls
(including disabled fieldsets), hidden inputs, invisible/inert content and closed
details descendants. Programmatic focus on a negative-tabindex control retains
its native starting position. Removing a focused row or descendant leaves BODY
active until the next key. A surviving row keeps the browser's native starting
point; its eligible neighbours mount synchronously for that key. Removed keys
retain the preceding logical row's position. Backward moves wait for conditional
details in a reached neighbour. This also covers partial control removal and
replacement with static content. Clicking static content inside a row retains
the clicked node's native starting position when BODY becomes active, mounting
eligible neighbours before the next Tab. External focus and pointer interaction
clear that retained position; the temporary neighbours depart when focus settles.

[virtualListIndex.ts](virtualListIndex.ts) owns measured prefixes and viewport
search. Adjacent index, spacing and native DOM tests use synthetic histories,
including 5,000 sparse rows, native entry/exit and bounded eligibility work.
The deterministic differential fixture renders identical keyed rows in a fully
mounted list and VirtualList, and compares actual native Tab focus sequences.
Default tests traverse complete small and sparse paths, a complete 64-row dense
nested-control path, and representative 5,000-row static viewports for every
control variant. Explicit identity, exclusion, removal, static/predicate
replacement, static Description clicks in both directions, conditional and
unconditional Description close/reopen, and pending-handoff regressions use the
same native DOM reference.
`test:ui:tab-full` adds the complete generated matrix across viewport positions
and both mutation sweeps. These include row/descendant/neighbour removal,
insertion, interactivity replacement and clearing dense, sparse and static lists,
with focus and viewports at both ends and the middle. Mutation traces reuse the
keyed fixture trees. Browser requests and each comparison have bounded deadlines;
both commands own one browser at a time and verify its absence on shutdown.

CountryPicker reads the recording-wide prepared land-owner catalogue through the shared pending-coverage polling. Its witness names and flag hours come from the lowest branch that observed positive ownership. A single compressed range spans all branches; the picker exposes "Land-owning countries" in parity builds.
