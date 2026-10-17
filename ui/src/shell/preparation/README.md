# Campaign preparation

The campaign library context menu opens **Preparation and storage** for loaded and archived recordings. The panel reads preparation state from the host and provides Prepare, Resume, Cancel, Refresh and Clear controls. Sections keep backend labels, exact decimal completed/total counts, and named failures. Only visible section rows render.

A recording prepared ahead of the user from the library shows **Paused** with its reason when it made way for preparation the user asked for, a newer recording, the free-space floor or a running game (`pauseReason`); it resumes automatically once the reason passes, and Resume or Cancel act on it at once.

`controller.ts` owns one visible panel and one pending request. It polls queued/preparing jobs, checks paused ones every `PAUSED_POLL_MS` (they resume without a notification), sleeps for terminal and idle states, rejects replies for another source/revision/request, and drops aborted replies. Archived recordings open a temporary source; closing the panel releases it without cancelling preparation. Loaded campaigns retain their existing binding.

Clear removes prepared data immediately. Open campaigns keep files already open until they close and prepare again. The panel displays the backend acknowledgement and never estimates progress or storage from timers.

`contract.ts` defines the narrow preparation request/reply boundary. Native and browser adapters validate the same replies; the browser bridge forwards preparation operations with their request ID.

Settings > Preparation stores whether opened recordings prepare automatically.
The choice defaults on. Disabling it stops automatic jobs and retains prepared
files; explicit Prepare and Resume remain available. Widgets never start an idle
job implicitly, so reading after Clear leaves its files cleared until Prepare.

Preparation replies also carry committed coverage hours by branch and each
family prepared on its own's coverage by family and branch. Prepared replies
name their coverage gates, so widgets refresh when their own families commit,
including while background preparation is paused or resumes. The playback
timeline shows this exact extent with recorded gaps excluded. Decode progress
never extends that marker before the corresponding prepared pages are committed.
