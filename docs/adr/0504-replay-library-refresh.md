# Refresh the replay library when either app changes its files

Both apps display a recording by its filename without the replay extension. The dashboard refreshes the library for changes by either app while it is open. A check finishes before another starts, and a change stamp becomes seen only after its directory scan succeeds. A failed scan can therefore be retried by the next check. Stable results keep memoised library views from rerendering on unrelated shell updates.

[Recording names](../../ui/src/dashboard/recordingNames.ts) · [Replay library](../../ui/src/dashboard/replayLibrary.ts)
