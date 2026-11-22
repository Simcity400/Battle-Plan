import { Lane, type LaneView } from './lane.ts';
import { fieldsView } from './views/fields.ts';
import { panelViews, readingStructure } from './panels/views.ts';
import { stretch } from './weakCpu.ts';
import type { FromLane, FromPull, ToLane, ToPull } from './laneMessages';
import { drawnViews } from './views/drawn.ts';

const post = (message: FromLane, transfer: Transferable[] = []) => self.postMessage(message, { transfer });
let lane: Lane<ImageBitmap> | null = null;
let rate = 1;
const fields = fieldsView(readingStructure, (product, panel, hour) => {
  const view = product === undefined ? undefined : panelViews[product];
  return view?.nextChange && panel.data !== undefined ? view.nextChange(panel.data, hour) : undefined;
});
const views = new Map<string, LaneView>([['fields', fields]]);
const viewOf = (kind: string): LaneView => {
  const held = views.get(kind);
  if (held) return held;
  const made = drawnViews(kind, widget => lane?.touch(widget)) ?? fields;
  views.set(kind, made);
  return made;
};

function start(index: number, lanes: number, port: MessagePort) {
  const made = new Lane<ImageBitmap>({
    lane: index, lanes, views: viewOf,
    crop: sources => Promise.all(sources.map(source => createImageBitmap(source as OffscreenCanvas))),
    present: (presentation, transfer) => post({ type: 'presentation', ...presentation }, transfer),
    release: bytes => port.postMessage({ release: bytes } satisfies ToPull),
    fail: reason => post({ type: 'failure', lane: index, reason }),
    later: run => { if (typeof requestAnimationFrame === 'function') requestAnimationFrame(() => run()); else setTimeout(run, 16); },
    stretch: ms => stretch(ms, rate, performance.now()),
  });
  port.onmessage = ({ data }: MessageEvent<FromPull>) => {
    if (data.kind === 'failure') { post({ type: 'failure', lane: index, reason: data.reason }); return; }
    try { made.receive(data.kind, data.bytes); }
    catch (error) { post({ type: 'failure', lane: index, reason: error instanceof Error ? error.message : String(error) }); }
  };
  return made;
}

self.onmessage = ({ data: m }: MessageEvent<ToLane>) => {
  if (m.type === 'init') { rate = m.rate; lane = start(m.lane, m.lanes, m.port); return; }
  if (m.type === 'options') { rate = m.rate; return; }
  if (!lane) return;
  switch (m.type) {
    case 'follow': lane.follow(m.sub, m.channel, m.product); break;
    case 'route': lane.bindRoute(m.sub, m.route, m.native); break;
    case 'unfollow': lane.unfollow(m.sub); break;
    case 'rank': lane.rank(m.sub, m.version); break;
    case 'mount': lane.mount(m.widget, m.subs, m.view, m.model); break;
    case 'unmount': lane.unmount(m.widget); break;
    case 'bind': lane.bind(m.widget, m.binding, m.model); break;
    case 'exact': lane.exact(m.sub, m.hour, m.data, m.refused); break;
    case 'stream': lane.stream(m.route, m.seek, m.target); break;
    case 'fail': lane.fail(m.route); break;
    case 'pointer': lane.pointer(m.widget, m.surface, m.point); break;
    case 'frame': lane.frame(m); break;
  }
};

self.onerror = event => { post({ type: 'failure', lane: -1, reason: String(event) }); };
