# 0701. Preserve battle-sound order and identity

Read the battle ambience manager through the located global. A readable null means it currently holds no battles. An unreadable global or unauthenticated object refuses the list. Traverse the spatial tree in pre-order, including incomplete levels, and preserve each node's stored battle order. Level order would reorder battles when a child has descendants. Each battle keeps its native floating-point center and radius bits and its ordered member province IDs, validated through the member objects.

The game keeps no persistent battle ID for this stream. The recorded ordinal follows the list, while the battle object's identity enables continuity through regroupings or merges it survives. An active main sound handle reports the game client's sounding state. Merges can keep the battle sounding for the recording client's camera, so the resulting list can depend on that camera. Combat updates and save loading change the list around the hourly boundary. Per-frame division counts and per-layer sound instances are outside this observation.

The sizing reader measures demand from live counts off-thread. Acquisition can grow worker buffers before reading a battle. Once fixed storage cannot fit one whole battle, count remaining demand without claiming to have read that payload. Retaining only complete rows prevents a short output suffix from becoming an apparently empty battle.

[Battle-sound read and sizing path](../../crates/engine-native-land/src/battle_sounds.rs) · [Ordering and refusal tests](../../crates/engine-native-land/src/battle_sounds/tests.rs)
