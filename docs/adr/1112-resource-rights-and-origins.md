# Resolve resource rights before computing delivered amounts

Resources join one-based database ordinals to recorded lexer tokens and the recorded content stack. A declaration gap means the database cannot be indexed reliably. ASCII-equivalent spellings that match multiple recorded tokens are refused by name. State inputs and building grants use lexer tokens, while rights use resource database ordinals. Resolve both canonical country identity and raw reference because an alias of the controller does not create a separate rights holder.

The state must first have an origin in its controller's own list. If the controller assigned a resource's rights to another country, use the first matching origin for that state's resource rights list. Use the controller's building overrides, oil factor and infrastructure factor, and the selected origin's route, local factor and efficiency. The route class is the resource origin's trade route. The state panel has its own controller-side resource amounts without this origin join.

Fixed-point multiplication truncates at every product. The nominal amount uses the full base and excludes damage, temporary amounts and efficiency. Delivered production clamps the base before applying it, subtracts damaged infrastructure's contribution, adds the temporary resource, and clamps production before efficiency and final cost. This can give a negative delivered amount when the resource only costs. Icons and hovers round to the nearest whole amount with halves away from zero.

A template listing, positive pool, building grant, cost, or additive gate can admit an icon. The admitted icon is withheld by name when the controller's origin is missing. Its rows show positive nominal amounts in database order. An icon uses a single window flag when all displayed rows assign rights to the same other country. Otherwise each row can carry its holder's flag. Below nominal or below zero is red, above nominal is green, and equal is neutral.

[Resource origins](../../crates/viewer-runtime/src/resource_origins.rs) · [Resource fixtures](../../crates/viewer-runtime/src/resource_origins/tests.rs)
