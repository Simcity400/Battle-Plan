# 0002. Use version changes to invalidate keyed read consumers

The battle logs can have several place and detail requests in flight. Each answer updates a keyed store in place so the painter can put it into visible rows as soon as it lands. Store references, including the references to the name and warning collections, can stay stable across answers.

`useKeyedReads` returns a `version` that advances as answers and failures land. Consumers that derive names, warnings and detail tooltips include the version in their memo and callback dependencies. Row renderers which use a callback's identity to detect updated content can then pick up changes even when their store reference is unchanged.

[Keyed read store](../../ui/src/dashboard/keyedReads.ts) · [Battle log](../../ui/src/widgets/battles/BattleLogView.tsx)
