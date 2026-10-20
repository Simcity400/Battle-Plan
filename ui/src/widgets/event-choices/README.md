# Event choices

## What it shows

One card per recorded delivery of a scripted or news event to a country, newest first by its firing date: the event's picture (behind **Game artwork**) and title, the receiving country with "from" its sender when one is recorded, when it fired and was chosen ("3 Mar 1938 → chosen 5 Mar 1938 · 2 days", or "same day"), and every authored option in the game's order ("A. …", "B. …") with the chosen one marked. An event still open at the playback date shows "Awaiting choice · N days open". A news event delivered to several countries is a card per receiving country. The header counts the cards shown once every page is known.

## Products it reads

[`event-choices`](../../../../crates/dashboard/src/products/event_feed/prepared/script.rs) supplies backward pages of occurrences from the event feed's prepared facts: each dispatch with the option executed for it, joined by the observer, never by parsing notes. Pages, prepared-prefix pending states, page demand and Live read bounds are the event feed's ([useScriptOccurrences.ts](../event-feed/useScriptOccurrences.ts), [scriptOccurrences.ts](../event-feed/scriptOccurrences.ts)); titles, pictures and option texts resolve through the recorded content stack.

## Settings

All countries by default, or manual/groups; campaign/follow/pin/range; **Country events**, **News events** and **Awaiting choice only**, also the filter chips; artwork and shared log options.

## Notable behaviour

- Cards are ordered by their own firing dates across every page: one read late is placed at its date, not where it was read. A card appears at the read that shows its firing; the choice and its date appear at the read that shows the choice. Seeking back removes them. An event's immediate option is not a choice.
- Dates are the game's own. While no choice is recorded, a break in the recording after the firing (a gap or an event loss) ends the claim: the card reads "not chosen by <the day before the break>" and "No choice recorded", and leaves the Awaiting choice filter.
- A choice whose firing was not recorded is its own card under its chooser, dated by the choice. It is kept whichever event kinds are chosen, since its kind is not recorded. An immediate effect whose firing was not recorded is no card: it is never a choice, nor one awaited.
- Chosen countries keep cards naming them in any role (receiver, sender, chooser) at that fact's hour. Countries keep their labels as they were at the firing.
- A title or option text the content does not resolve reads "Title unavailable" or "Option text unavailable", named by Content identity and counted in the diagnostics. Clicking a card does nothing; there is no game view yet.
