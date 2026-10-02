# Flow diagrams

`FlowDiagram` draws source and target nodes joined by ribbons. The widget owns their meaning and sums the countries beyond `flowCapacity` into its last node; `layout.ts` owns geometry, motion, hit testing and keyboard stops.

Pass a memoized `tooltip` callback with every input it reads. `FlowTooltipCache` reuses a still hover's content until its hit or callback changes, then refreshes it. Keyboard hover follows the same callback changes. There is no separate revision list to keep in step with the widget's model.

The widget fits its tooltip with the shared [tooltip kit](../tooltip.ts), and supplies its recorded-hour names and flags with [useHoverLabels](../../dashboard/useHoverLabels.ts). See the [chart rules](../README.md) and [Trade flows](../../widgets/trade-flows/README.md) for the consumer.
