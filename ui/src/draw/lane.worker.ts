import { fieldsView } from './views/fields.ts';
import { panelViews, readingStructure } from './panels/views.ts';
import { stretch } from './weakCpu.ts';
import type { FromLane, ToLane } from './laneMessages';
import { drawnViews } from './views/drawn.ts';
import { laneDispatch, macrotaskSchedule } from './laneDispatch.ts';

const post = (message: FromLane, transfer: Transferable[] = []) => self.postMessage(message, { transfer });
const fields = fieldsView(readingStructure, (product, panel, hour) => {
  const view = product === undefined ? undefined : panelViews[product];
  return view?.nextChange && panel.data !== undefined ? view.nextChange(panel.data, hour) : undefined;
});

const schedule = macrotaskSchedule();

const dispatch = laneDispatch(post, {
  views: (kind, wake) => kind === 'fields' ? fields : drawnViews(kind, wake) ?? fields,
  crop: sources => Promise.all(sources.map(source => createImageBitmap(source as OffscreenCanvas))),
  later: run => { if (typeof requestAnimationFrame === 'function') requestAnimationFrame(() => run()); else setTimeout(run, 16); },
  ...schedule ? { schedule } : {},
  stretch: (ms, rate) => stretch(ms, rate, performance.now()),
});

self.onmessage = ({ data }: MessageEvent<ToLane>) => dispatch(data);

self.onerror = event => { post({ type: 'failure', lane: -1, reason: String(event) }); };
