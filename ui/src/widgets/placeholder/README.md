# Placeholder

## What it shows

An empty panel reserving space in a saved layout until another widget replaces it.

## Products it reads

None. The definition marks it as a non-data widget; it has no country or recording-specific body.

## Settings

The frame provides title and size/layout controls. There are no widget-specific options or playback-dependent values.

## Notable behaviour

- Keep it independent of data clients and campaign availability.
- Rename, move and resize through the shared layout framework; replacing it is a layout edit.
- Metadata and registration follow the [widget contract](../README.md#contract).

## Files

| File | Responsibility |
|---|---|
| [definition.ts](definition.ts) | Non-data metadata and size defaults. |
| [index.tsx](index.tsx) | Static Empty panel renderer and registration. |
