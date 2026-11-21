# 0505. Apply native window events only to the window they describe

A map pop-out has a window generation and a viewer generation within it. Events carry the host sequence, so an event for an older state or a previous window cannot affect the current view. A whole state includes any unrecovered window error. Clock and settings subscriptions are keyed to the viewer as well as the window, so a restarted viewer receives the current playback hour and map options again. Both event listeners are installed before requesting the initial host state. A recording explicitly unloaded by the user can close a pop-out that shows it. Reconnection does not unload it.

The desktop app accounts for native child window z-order when showing menus and the sidebar peek. A docked panel narrows the map layout rather than occluding the child. Layout is measured before painting. The recovery flow keeps a shell-owned error visible until the operation succeeds or the user chooses a new view. A rejected hide or resize cannot revive the failed child, and a newer resize or tab intent replaces any waiting one. A late command-line recording is adopted only before the user has selected a dashboard recording.

Map switches display the greatest received sequence across host events, snapshots and setter replies. The host persists them and sends them to its windows. Ctrl+F3 toggles diagnostics, keeping the game's standalone F3 map mode. Returning a map operation to the previous view restores that view's native ownership. A checksum failure on a recording does not identify missing blocks, which are checked only as playback reads them.

A map's canonical country slot follows the country through tag changes. A pinned choice is protected from a map click. The acknowledged click sequence records which native choice the dashboard applied, so an old native state cannot override a newer selection.

Shell-only state keeps the dashboard and header isolated from those renders. The development browser also supports an HTTP LAN preview. UUID creation falls back to `getRandomValues` where the context does not expose `randomUUID`, retaining random RFC 4122 version 4 identifiers.

[Pop-out](../../ui/src/shell/popout.ts), [Native viewer](../../ui/src/shell/useNativeViewer.ts), [Map switches](../../ui/src/shell/mapSwitch.ts), [Desktop shell](../../ui/src/shell/App.tsx), [Recording notices](../../ui/src/shell/recordingNotice.tsx)
