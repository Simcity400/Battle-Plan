# Decryption

## What it shows

Recorded decryptor → target attempts, progress/status and active bonus countdowns.

## Products it reads

[`decryption`](../../../../crates/dashboard/src/products/decryption.rs) supplies pair histories; [cryptology.rs](../../../../crates/dashboard/src/cryptology.rs) owns native threshold/progress arithmetic. Shared country facts supply dated endpoint looks.

## Settings

All/manual/group countries, campaign/follow/pin/range; order by progress/decryptor and filter countries as decryptor or target.

## Notable behaviour

- Progress uses same-hour recorded target strength and content defines. Unknown threshold keeps exact stored amount and Progress unknown; no guessed denominator.
- Started belongs to the current attempt, not the pair’s first lifetime start. Pausing retains it; expiry/clearing can end it. Post-gap activity and uncertain breaks use By bounds; future observations stay hidden. Jump offers attempt-start/break observations.
- Daily linear runs reproduce recorded integer hours with native arithmetic and split at mismatches/state changes/gaps; never interpolate an invented observation.
- Manual choices follow native holder aliases at endpoint identity hours. Unrepresentable tag bytes retain IDs/data with diagnostics and request no artwork.
- Window only drawn rows. Pending endpoint looks can hold only within the same pair/tag stretch; backward seeks and completed missing reads clear them. Known empty data differs from missing capture.

## Files

| File | Responsibility |
|---|---|
| [DecryptionWidget.tsx](DecryptionWidget.tsx) | Virtual rows, progress meters and dated labels. |
| [decryptionData.ts](decryptionData.ts), [decryptionCountries.ts](decryptionCountries.ts) | Validation, attempts, exact runs and holder selection; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [decryption.css](decryption.css) | Options, registration and layout. |
