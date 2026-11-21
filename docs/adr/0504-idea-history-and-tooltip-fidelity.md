# Format idea effects using recorded modifier definitions

An idea history is read once per branch selection. Idea spans, unread spans and identity boundaries provide the change hours, so a following widget only needs to render when what it can show changes. Each tooltip uses the recorded holding hour when naming target countries. Keep a missing localisation or artwork reason in the tooltip detail and retain explanatory text when artwork is unusable. Validate effect arrays before the shared builder reads them from any product.

Use the game's modifier formatting flags and ordering. Format fixed-point values by truncation. Respect sign, precision, units, neutral colouring, zero suppression, and clamp flags. List built-in modifiers by index before scripted modifiers, omit hidden lines, and display the recorded final value of a repeated modifier. An unavailable modifier format leaves the exact value visible with a muted unknown label and lookup key. A feature-gated effect gets a note since the recording does not certify that the feature was enabled. Unknown effects with no recorded definition are omitted, retaining the reason in the reply.

Keep the traits, general effects, targets, research and equipment in game order. Equipment stats use the authored values when the recording lacks the engine format table. Limit visible effects by line budget, taking wrapping labels, dividers and the hidden-count notice into account. The accessible description still reads the tooltip content.

[Idea history](../../ui/src/dashboard/ideaHistory.ts) · [Tooltip reply validation](../../ui/src/dashboard/ideaHistoryReply.ts) · [Tooltip formatting](../../ui/src/dashboard/ideaTooltip.ts)
