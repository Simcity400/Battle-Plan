# Flow diagrams

[FlowDiagram.tsx](FlowDiagram.tsx) draws source/target nodes joined by ribbons. Widgets supply meaning, exact tooltip amounts and dated labels; [layout.ts](layout.ts) owns geometry, motion, hit testing and keyboard stops. See the [chart contracts](../README.md) and [Trade flows](../../widgets/trade-flows/README.md).

## Data and fitting

Give nodes/links stable IDs and `order` keys. Nodes sort by size with deterministic ties; `last` keeps a remainder last, and ribbon order stays stable within nodes. `onCapacity` reports the capacity for the current height: the caller sums overflow into its last node without losing amounts. `scale` and `slots` can keep one scale throughout a campaign.

A changed `stretch` or reduced motion snaps geometry. `playbackHour` is the last hour included in cumulative data; moving it backward removes future geometry immediately. `hour` stamps the drawn frame used by `onPick`.

## Interaction

Memoize `tooltip` with every input it reads. [tooltipCache.ts](tooltipCache.ts) refreshes still pointer/keyboard hover when the hit or callback changes. Dated country facts use [useHoverLabels](../../dashboard/useHoverLabels.ts).

With `hour` and `onPick`, click/Enter/Space activates the actual drawn node/ribbon and its supplying hour, retaining caller selection metadata. With `onLabelPick`, a node's drawn flag and name are pointed at as the node (tooltip, menu) and a click on them calls it instead; without it they are not. The context-menu key and Shift+F10 keep the menu action; without `onPick`, Enter opens that menu too. Keyboard stops and pointer hits use the drawn geometry.

Use explicit `countryFlag` for native countries, retaining grey placeholders and dated refusal reasons; aggregates remain flagless. Fitted tooltips decode only shown flags. Scrollable breakdowns retain raw rows and a stable `scrollId`; wheel/Page Up/Down go through shared owner-scoped scrolling, whose mounted viewport decodes row flags. An explicit dated tooltip header takes precedence over the node fallback. See [tooltip.ts](../tooltip.ts) for ownership and full spoken text.
