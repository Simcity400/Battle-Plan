import { fieldsView } from './views/fields.ts';
import { panelViews, readingStructure } from './panels/views.ts';
import { stretch } from './weakCpu.ts';
import type { FromLane, ToLane } from './laneMessages';
import { drawnViews } from './views/drawn.ts';
import { cropSurfaces, laneDispatch, macrotaskSchedule, type LaneSide } from './laneDispatch.ts';

const fields = fieldsView(readingStructure, (product, panel, hour) => {
  const view = product === undefined ? undefined : panelViews[product];
  return view?.nextChange && panel.data !== undefined ? view.nextChange(panel.data, hour) : undefined;
});

export function startLaneWorker(
  post: (message: FromLane, transfer?: Transferable[]) => void,
  overrides?: Partial<LaneSide>,
) {
  const schedule = overrides && 'schedule' in overrides ? overrides.schedule : macrotaskSchedule();
  return laneDispatch(post, {
    views: overrides?.views ?? ((kind, wake) => kind === 'fields' ? fields : drawnViews(kind, wake) ?? fields),
    crop: overrides?.crop ?? (sources => cropSurfaces(sources)),
    later: overrides?.later ?? (run => { if (typeof requestAnimationFrame === 'function') requestAnimationFrame(() => run()); else setTimeout(run, 16); }),
    ...schedule ? { schedule } : {},
    ...overrides?.clock ? { clock: overrides.clock } : {},
    stretch: overrides?.stretch ?? ((ms, rate) => stretch(ms, rate, performance.now())),
  });
}

if (typeof self !== 'undefined') {
  const post = (message: FromLane, transfer: Transferable[] = []) => self.postMessage(message, { transfer });
  const dispatch = startLaneWorker(post);
  self.onmessage = ({ data }: MessageEvent<ToLane>) => dispatch(data);
  self.onerror = event => { post({ type: 'failure', lane: -1, reason: String(event) }); };
}
