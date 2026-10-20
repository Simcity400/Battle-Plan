# Industrial organisations

## What it shows

A moment view (Follow or Pin) of one country's industrial organisations, largest first: icon and in-game name, size, funds, traits unlocked and the selected policy (with its cooldown while one runs). There is no units column and no chart. A row opens the game's policy screen for that organisation in the drill, drawn by the game-window renderer.

## Products it reads

[`production-catalogue`](../../../../crates/dashboard-economy/src/products/production_catalogue.rs) in its `organisations` view (family `production_catalogue`): the complete roster and each organisation's stored state and selected policy from the daily production slot. Names and icons use Production lines' organisation selectors through the recorded content; the policy name is the country's stored policy name.

## Notable behaviour

- Ranking by size, then funds, then native reference is the app's order, not the game's. An organisation whose state was not read shows em dashes and an unknown policy, never zero or "None".
- The row action opens the policy chooser through the game-window seam ([gameWindows.ts](../../dashboard/gameWindows.ts)): the widget passes a semantic selection (recording, branch, hour and the full organisation reference as its decimal u64, the spelling every product uses) and the dashboard client; the window resolves the holder itself. The game-window renderer registers it ([gameWindowRegistry.tsx](../../components/gameWindowRegistry.tsx)) as the organisation's policy screen (`mioPolicies`) at the drill's hour; if no renderer were registered, rows would do nothing and their tooltip would say the screen is not available yet. The open window's own diagnostics (refused controls, unsupported effects, unavailable artwork) join the card's beside the catalogue warnings.
- Columns hide as the widget narrows (Traits, then Policy); their values stay in the row tooltip.
