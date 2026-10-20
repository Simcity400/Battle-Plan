# Volunteers & expeditionaries

**What it shows:** Volunteer divisions and expeditionary forces from each sender to each host at the shown hour, drawn exactly as Trade flows draws its flows. A node's flag or name opens that country's partners in the shared drill.

**Data it reads:** `volunteers-expeditionaries` for the chosen **Kind** and the chosen countries' tags, and `country-groups` for the identity history and group memberships: sparse sender-to-host links of the sender's listed divisions held by each host, from the army preparation.

**Settings:** **Kind** (also the caption switch); With countries chosen, show; Countries per side (the rest sum into Other); Ribbon width; Breakdown opens.

**Notable behaviour:**

- A link whose count is not known at an hour is counted in diagnostics, never drawn as zero. There is no "ever sent" total: summing deployed divisions over time is not a count of divisions sent.
- Air volunteers and volunteer caps are not shown: the recording has no proven classifier or cap for them.
- Nodes are native holders (tag, slot and country id): when a tag passes to another country, its node fades out and the new holder's fades in, never gliding from one to the other, and an open drill keeps the holder clicked, named as it was then. The country choice is the shared one (`countryMatch` and `countryMatchFilter` in [groups.ts](../../dashboard/groups.ts)): countries count by native holder through the recorded identity history, so a reused tag or a rename never changes whose forces count, under every membership setting, and a group by hour counts a member only while it belongs, at the hour shown and in the campaign scale. Group membership not recorded at an hour reads unavailable, and a group recorded without members reads empty; neither is a country missing from the campaign, and neither sends a request. While the groups are still being prepared, the widget says so instead of calling a country unavailable. The request names every tag the chosen holders carry in the branch; only All asks for every link.
