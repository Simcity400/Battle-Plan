# 1393. Desktop IPC has one path: the IPC custom protocol

The UI reaches the desktop host only through Tauri's IPC custom protocol (`http://ipc.localhost`). The app replaces Tauri's invoke script with its own (`Builder::invoke_system`). That script has no postMessage fallback.

Tauri's own script falls back for good once any custom-protocol fetch rejects, and then sends every call over `window.ipc.postMessage`. That path is far slower: replies come back as evaluated script. A reload, including the e2e sweep's `Page.reload` and the user pressing F5, cancels the page's in-flight IPC fetches. The unloading page then took that as the protocol being blocked and re-sent each call over postMessage. The commands ran twice, and its `listen` calls registered event listeners for a page that no longer existed.

- A request that fails while the page is live rejects that invoke with a named error ("BattlePlan couldn’t reach its desktop host for <command>: …") and logs it. The next invoke still uses the custom protocol. Dashboard queries show that sentence (`queryFailure` takes the string rejection as a shown error).
- A request lost after `pagehide` is dropped. The page that asked is gone, and the new page makes its own requests.

The script runs in every webview the app opens, in development and in the packaged app alike.

[Invoke script](../../crates/desktop/src/ipc.js)
