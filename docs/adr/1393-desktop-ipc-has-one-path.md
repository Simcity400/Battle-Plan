# 1393. Desktop IPC has one path: the IPC custom protocol

The UI reaches the desktop host only through Tauri's IPC custom protocol (`http://ipc.localhost`). The desktop app and the recorder replace Tauri's invoke script with this one (`Builder::invoke_system`). That script has no postMessage fallback.

Tauri's own script falls back for good once any custom-protocol fetch rejects, and then sends every call over `window.ipc.postMessage`. That path is far slower: replies come back as evaluated script. A reload, including the e2e sweep's `Page.reload` and the user pressing F5, cancels the page's in-flight IPC fetches. The unloading page then took that as the protocol being blocked and re-sent each call over postMessage. The commands ran twice, and its `listen` calls registered event listeners for a page that no longer existed.

- A request that fails while the page is live rejects that invoke with a named error ("BattlePlan couldn’t reach its desktop host for <command>: …") and logs it. The next invoke still uses the custom protocol. Dashboard queries show that sentence (`queryFailure` takes the string rejection as a shown error).
- A request lost after a `pagehide` that unloads the page is dropped. The page that asked is gone, and the new page makes its own requests. A page kept in the back/forward cache (`persisted`) still hears its failures, so no invoke is left waiting.
- A plain array payload is sent as those bytes (`Uint8Array.from`).
- Nothing is retried on another path. Each failure below is shown by name with its own Try again, which repeats only that failure's step.

## Closing asks the live page

A failed reply can come from a request the host still applied, so any copy of the page's unsaved state kept in the host can be out of order with the page. The host keeps none. The live page is the only judge of unsaved work, and it judges at the moment it is asked.

- The main page subscribes once with a Tauri channel and a sequence number (`watch_close_requests`). The host keeps the newest subscription; the same registration sent again after a lost reply changes nothing, and an older one arriving late is ignored. A superseded subscription in the page is inert. A question still waiting when a newer subscription arrives is asked of it.
- A window close is always held and sent to the page. Saved work leaves (`leave_app`); unsaved work shows the prompt, and the user's choice leaves or stays. A page that never subscribed guards nothing, so the window closes.
- The update path asks the page a question and waits for its answer (`answer_close_question`): the installer's close request and the other app's busy query on the control pipe, and an install the page didn't confirm, at its final step. Unsaved work refuses with the update message.
- "Discard and restart" confirms as part of its install request (`install_update_leaving`). Only that install trusts the confirmation; a window close during it still asks the page.
- A page that reloads stops being asked; a question it left unanswered is refused by name, and the new page subscribes again. A page whose process exits (WebView2's `ProcessFailed`, browser or render process exited) can't answer, so its questions are released and the next close goes through. A page that is alive is always asked, and no timer decides for it.

## The recorder panel's prompts

- The panel listens for prompts before it reads its opening state (`panel_opened`). A failed listen or a failed read is shown with Try again, and retries only that step.
- The recorder queues each Quit or Discard prompt with an id and keeps it until a panel handles or dismisses it, which retires it by id; a newer prompt stays. A panel that asks again, or a reloaded one, is given the prompts still pending, so a prompt the user cancelled isn't shown again.

The script runs in every webview either app opens, in development and in the packaged app alike. It is adapted from Tauri's IPC protocol script, and Tauri's licence ships in the third-party notices.

[Invoke script](../../crates/desktop/src/ipc.js)
