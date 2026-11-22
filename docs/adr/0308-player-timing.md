# 0308. Derive player activity and elapsed time from capture coverage

Effective APM and elapsed time use the command index's count and unpaused-tick prefixes. A trailing-window answer subtracts prefixes at the two window edges for the intersecting actor versions, avoiding a walk of commands or hours. A series tracks both the human's stored user string and the country and capture span it played in, so the same player is joined across reconnects and reattachments. A timeline counts each capture only on the slice it keeps. The branch's first and last capture edges bound it. The order numerator and observed-time denominator both exclude unknown periods, including partial capture seams.

The request, head and chart paths share the index. A follow lays out the player versions and their spans once, then queries differences at the playhead. A tile's 256 level bins calculate a separate rate over each bin's period. They read the prefix edges for their intersecting series parts and check the covered span once for coverage. The campaign clock uses the same recorded, playing and paused time evidence from the branch start.

[Player activity](../../crates/dashboard-players/src/products/player_apm.rs) · [Head and chart activity](../../crates/dashboard-players/src/products/player_apm_head.rs)
