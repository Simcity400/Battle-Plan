# Diplomatic pacts & proposals

**What it shows:** **Proposals**: each country's queued diplomatic proposals as dated cards, sender and recipient (and the original route when it differs), stored date and when the queue stopped listing them. **Pacts**: every recorded relation (guarantees, non-aggression, access, lend-lease, embargoes and the rest) with its endpoints, stored and observed dates.

**Data it reads:** `diplomacy-state` (`view: 'proposals'` and `view: 'pacts'`). Relation kinds are named from the recorded content.

**Settings:** **View** (also in the header, and its own picker card "Diplomatic pacts"); Proposals: **Waiting only** and the log options; Pacts: **Kinds** (one choice per recorded kind), **Ended pacts** shown or hidden, the table's options. Countries narrow to items where a chosen country takes any role.

**Notable behaviour:**

- The recording has no proposal kind or outcome, so neither is shown: a proposal leaving the queue proves no answer.
- A pact's presence is what the recording lists; its own stored dates are shown where it carries them, else the reads bound it ("by", "gone by").
- Countries are named as they were when the item was last listed (or at the hour shown, if earlier), only for mounted rows; a name still being read says so.
