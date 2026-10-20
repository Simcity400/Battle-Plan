# Division service record

## What it shows

One division's recorded career and awards, newest first, in the division log's row look, under a header line: its name, then template, medal total, experience level and kill counter.

## Products it reads

[`division-record`](../../../../crates/dashboard-army/src/products/division_careers.rs) at the shown hour; the Division choice reads `division-careers` for the chosen countries at the campaign's last hour.

## Settings

Division (a searchable choice of the chosen countries' divisions); Show (Career and awards, Career, Awards); the shared log options; Campaign (default), Follow or Pin.

## Notable behaviour

- Entries keep their native order and repeats; descriptions use the game's own wording through the shared [division history](../../../../crates/dashboard-text/src/division_history.rs) rules, and an entry that cannot be described keeps its date with *Entry description unavailable*.
- Each header part missing on its own reads "—", never 0. Medal art is not drawn yet; awards use the medal's name.
- A division not in service at the shown date says so.
- Right-click jumps to an entry's date when it lies inside the recording.

## Files

| File | Responsibility |
|---|---|
| [RecordView.tsx](RecordView.tsx) | Header and dated record, shared with the Most decorated divisions drill. |
| [careersData.ts](careersData.ts) | Validation, rows, header text, tooltips, choices and Pin; adjacent tests. |
| [ServiceRecordWidget.tsx](ServiceRecordWidget.tsx), [index.ts](index.ts) | The standalone widget and its Division choice. |
