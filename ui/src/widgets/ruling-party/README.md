# Ruling party

One country's politics over time (mock-up politics-2).

- **Leader header:** the leader at the moment shown.
  - Their portrait sits in a ring coloured by the ruling ideology (grey when the ruling party is not known).
  - Then their name, and the line "sub-ideology · ruling party · its popularity". The party is named as the game names it ("NSDAP"); when that cannot be known it is left out.
  - When the leader is unknown, the header says why instead of guessing: "Leader not recorded", "No leader", "Leader not identified", or "Leader's name not known". A portrait that cannot be shown becomes a silhouette; the reason is on its tooltip, which keyboard focus also opens.
- **Parties:** each party's popularity is a stacked band in its ideology's own colour from content, a mod's ideologies included. Without a content colour, a party takes the categorical palette in order. The legend wraps when there are many ideologies.
- **Ruling party first:** the party ruling at the moment shown is the bottom band and the legend's first entry (mock-ups politics-1/2). The legend lists names only.
- **Badges** along the top mark changes of government, elections and changes of leader. Each kind has its own toggle. A change next to a recording gap is not shown as a change. A rename or a new portrait is still the same leader.
- **Tooltip:** the leader, their ideology and party (the party's own name, "Not known" when that cannot be known), then every party's popularity.
- **Right-click** jumps to that moment.
- In whole-campaign playback, the bands and badges stop at the playhead unless **Show after the playhead** is on.

## Data

**Product `party-history`** (`crates/dashboard/src/products/party_history.rs`) supplies:

- runs of each party's exact popularity (`PoliticalParties`, fixed-point percent);
- ruling runs;
- election hours (when `Politics.last_election` changes);
- content's ideology names and colours;
- leader runs;
- runs of the ruling party's name.

**The leader** is the ruling party's first country leader, as `CountryAppearance.leader_ideology` and `leader_character` record it (format 54). It is named through `Characters` the way an advisor is: the saved name first, then the authored name of its script token. The sub-ideology is named from localisation.

**The portrait** is the one the game's leader views ask for (`SetCountryLeaderPortrait@140B38140`: role civilian, size large). It comes from the recorded `CharacterPortraits` holder through `replay::rows::portrait_entry`: the civilian large entry, else the first large entry. It is used only when the entry is set and content has its sprite, or its texture loads. Otherwise the game generates a portrait from inputs the recording lacks, so none is shown.

**The party's name** is the game's (`GetName@14118F860`, format 55). A party renamed by `set_party_name` shows its stored key, localised (a key localisation lacks shows as itself, as in the game). Otherwise the game builds `<tag>_<ideology group>_party`, then tries the country's original tag (`CountryAppearance.name_tag`), then falls back to the ideology group's own name. A name is not known when the tag or original tag was not recorded, or when its text needs runtime scope. The legend keeps ideology group names, as signed off.

## Not yet

- **The cause of a change of government** (election, coup, civil war) is not recorded. Badges say only that it happened.
