# 1393. Desktop IPC has one path: the IPC custom protocol

The UI reaches the desktop host only through Tauri's IPC custom protocol (`http://ipc.localhost`). The desktop app and the recorder replace Tauri's invoke script with this one (`Builder::invoke_system`). That script has no postMessage fallback.

Tauri's own script falls back for good once any custom-protocol fetch rejects, and then sends every call over `window.ipc.postMessage`. That path is far slower: replies come back as evaluated script. A reload, including the e2e sweep's `Page.reload` and the user pressing F5, cancels the page's in-flight IPC fetches. The unloading page then took that as the protocol being blocked and re-sent each call over postMessage. The commands ran twice, and its `listen` calls registered event listeners for a page that no longer existed.

- A request that fails while the page is live rejects that invoke with a named error ("BattlePlan couldn’t reach its desktop host for <command>: …") and logs it. The next invoke still uses the custom protocol. Dashboard queries show that sentence (`queryFailure` takes the string rejection as a shown error).
- A request lost after a `pagehide` that unloads the page is dropped. The page that asked is gone, and the new page makes its own requests. A page kept in the back/forward cache (`persisted`) still hears its failures, so no invoke is left waiting.
- A plain array payload is sent as those bytes (`Uint8Array.from`).
- Nothing is retried on another path. Each failure below is shown by name with its own Try again, which repeats only that failure's step.

## Closing asks the live page

A failed reply can come from a request the host still applied, so any copy of the page's unsaved state kept in the host can be out of order with the page. The host keeps none. The live page is the only judge of unsaved work, and it judges at the moment it is asked.

- The main page subscribes once with a Tauri channel and a sequence number (`watch_close_requests`). The host takes the channel's id (`JavaScriptChannelId`) and builds a channel only for a newer subscription: a dropped Tauri channel ends its page callback, so the same subscription sent again after a lost reply, or an older one arriving late, builds and drops nothing. A superseded subscription in the page is inert.
- The host asks with a fresh id each time it sends a question, and the page answers from its state at that moment (`answer_close_question`). A question still waiting when a newer subscription arrives is sent to it with a new id, so the superseded page's late answer settles nothing. The page reads its layouts from the dashboard's latest drafts and its theme from its latest edit, not from the copy the dashboard reports after it renders.
- A question is about a `close` (the window's close button), a `leave` (the installer's close request on the control pipe, and an install at its last step) or a `query` (the other app's busy query). Unsaved work keeps the window and shows the prompt, or refuses the update with the update message. A busy query never changes the page.
- Agreeing to leave locks editing first: the page shows that BattlePlan is closing until it exits or the host sends `stay` for that question (an installer that couldn't start). An answer the host no longer waits for lets the page go on.
- "Discard and restart" is the page's own confirmation, given as its answer when the install asks at its last step. Cancel withdraws it before editing resumes, so a question asked after Cancel is refused; once the page has answered, the dialog can't be cancelled. "Discard and close" locks editing, then leaves (`leave_app`).
- Before a page starts there is nothing to keep, and a close goes through. A starting page (wry's `Started`, at WebView2's `ContentLoading`) is a new document: questions the old one left unanswered are refused by name, update questions are refused by name until the new page subscribes, and a window close waits for the subscription. A page whose process exits (WebView2's `ProcessFailed`, browser or render process exited) held nothing that can be saved, so its questions are released and closes go through.
- No timer decides for the page. WebView2 reports a page that stops answering input (`RenderProcessUnresponsive`), and each report refuses the update questions waiting on it by name. Such a report, or a second close while the page hasn't answered the first, opens one native choice: keep waiting, or close BattlePlan and lose anything unsaved.
- The control pipe answers a close request with the refusal's own words, and setup shows them ([ADR 0006](0006-installer-coordination.md)).

## The recorder panel's prompts

- The recorder keeps each tray Quit or Discard prompt with an id and the key of the session it asks about, and publishes the pending one in its status. A panel, new or reloaded, shows what the status holds and never retires a prompt itself. Cancel retires prompts by id in the recorder, with only the Quit intent those prompts carry. A Discard asked about one session is dropped when another begins; a pending Quit asks again about the new one.
- Save and Discard name the session the user saw (`stop_recording` with its key). Under its lock the recorder refuses a session that is no longer live or already stopping, and the same request sent again changes nothing. A prompt stays until the recorder answers; a failure is shown in the prompt with Try again and keeps a compact panel open.
- The panel reads its opening state (`panel_opened`) once; a failed read is shown with Try again.

The script runs in every webview either app opens, in development and in the packaged app alike. It is adapted from Tauri's IPC protocol script, and Tauri's licence ships in the third-party notices.

[Invoke script](../../crates/desktop/src/ipc.js)
