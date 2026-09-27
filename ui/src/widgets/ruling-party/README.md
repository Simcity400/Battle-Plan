# Ruling party

One country's politics over time (mock-up politics-2): each party's popularity as stacked bands in its ideology's own colour from content (a mod's ideologies too; without a content colour, the categorical palette in order), with a legend that wraps for many ideologies. The party ruling at the moment shown is the bottom band and the legend's first entry (mock-ups politics-1/2); it is named in the header and the tooltip, and the legend lists names only. Changes of government and elections are badges along the top (a change next to a recording gap is not shown as one). Right-click jumps to that moment. In whole-campaign playback the bands and badges stop at the playhead unless **Show after the playhead** is on.

## Data

- **Product `party-history`** (`crates/dashboard/src/products/party_history.rs`): runs of each party's exact popularity (`PoliticalParties`, fixed-point percent), ruling runs, election hours (`Politics.last_election` changing), and content's ideology names and colours.

## Not yet

- **Country leader header** (portrait with a ring in the ruling ideology's colour, name, ideology): the recording holds each character's leader roles by the engine's numeric ideology key, but neither that key's ideology nor which leader heads the ruling party is recorded. It needs new capture (collected in the master spec).
- The **cause** of a change of government (election, coup, civil war) is not recorded; badges say only that it happened.
