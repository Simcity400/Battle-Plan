# 0805. Rebuild political membership using the saved writer’s identity

The game saves character status through a map keyed by character reference. Compare membership without native order or duplicates and reject null recorded references. Party leaders retain vector order. Join a saved party through its recorded ideology token and resolve the saved character role by the recorded map key even when the role’s ideology changes. A null parent has no role ID. Civil-war targets retain array order and resolve raw references through their linked tags.

Available-advisor membership uses the writer’s advisor flags and the saved role idea, including an empty key. It compares only countries read at the oracle hour, unless a full read or older unsliced recording proves wider coverage. The save omits the primary ordering word, so several case-folded ideas may prove membership without establishing the representative. Faction slots compare in their saved order. Their idea must match the advisor’s saved roles, with exact equality only when one role identifies it.

The advisor token set is reconstructed rather than serialized. A load populates it from active characters with the advisor flag and from all retired roles. The first spelling wins when the folded key and equality check agree. At an observed load’s first sampled hour, compare the reconstructed set exactly. At other hours, dynamic country replacement can leave tokens behind after both rosters are cleared. Treat those tokens as a limitation only when their known holders are no longer listed. Missing tokens, tokens with no role, and tokens still held by listed characters remain differences.

[Political and character checks](../../crates/recorder-oracle-politics/src/politics.rs)
