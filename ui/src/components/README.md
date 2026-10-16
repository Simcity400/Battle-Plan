# Shared components

Reusable controls, popovers, tooltips, country choices and flags. Shared appearance
and control conventions live in the [frontend guide](../../README.md#shared-appearance-and-controls).

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
