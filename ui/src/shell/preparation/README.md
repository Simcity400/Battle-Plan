# Campaign preparation

The campaign library context menu opens **Preparation and storage** for loaded and archived recordings. The panel reads preparation state from the host and provides Prepare, Resume, Cancel, Refresh and Clear controls. Sections keep backend labels, exact decimal completed/total counts, and named failures. Only visible section rows render.

`controller.ts` owns one visible panel and one pending request. It polls only queued/preparing jobs, sleeps for terminal/idle states, rejects replies for another source/revision/request, and drops aborted replies. Archived recordings open a temporary source; closing the panel releases it without cancelling preparation. Loaded campaigns retain their existing binding.

Clear removes prepared data immediately. Open campaigns keep files already open until they close and prepare again. The panel displays the backend acknowledgement and never estimates progress or storage from timers.

`contract.ts` defines the narrow preparation request/reply boundary. Native and browser adapters validate the same replies; the browser bridge forwards preparation operations with their request ID.

Settings > Preparation stores whether opened recordings prepare automatically.
The choice defaults on. Disabling it stops automatic jobs and retains prepared
files; explicit Prepare and Resume remain available. Widgets never start an idle
job implicitly, so reading after Clear leaves its files cleared until Prepare.

Preparation replies also carry committed coverage hours by branch. The playback
timeline shows this exact extent with recorded gaps excluded. Decode progress
never extends that marker before the corresponding prepared pages are committed.
