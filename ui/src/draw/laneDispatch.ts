import { Lane, type LaneHost, type LaneView } from './lane.ts';
import type { FromLane, FromPull, ToLane, ToPull } from './laneMessages';

export interface LaneSide {
  views(kind: string, wake: (widget: number) => void): LaneView;
  crop: LaneHost<ImageBitmap>['crop'];
  later(run: () => void): void;
  schedule?: LaneHost<ImageBitmap>['schedule'];
  stretch?(ms: number, rate: number): void;
}

export function laneDispatch(post: (message: FromLane, transfer?: Transferable[]) => void, side: LaneSide): (message: ToLane) => void {
  let lane: Lane<ImageBitmap> | null = null, rate = 1;
  const views = new Map<string, LaneView>();
  const viewOf = (kind: string) => {
    let held = views.get(kind);
    if (!held) views.set(kind, held = side.views(kind, widget => lane?.touch(widget)));
    return held;
  };
  const start = (index: number, lanes: number, port: MessagePort) => {
    const made = new Lane<ImageBitmap>({
      lane: index, lanes, views: viewOf, crop: side.crop, later: side.later,
      ...side.schedule ? { schedule: side.schedule } : {},
      present: (presentation, transfer) => post({ type: 'presentation', ...presentation }, transfer),
      release: bytes => port.postMessage({ release: bytes } satisfies ToPull),
      hit: (widget, key) => post({ type: 'hit', widget, key }),
      served: frame => post({ type: 'served', ...frame }),
      fail: reason => post({ type: 'failure', lane: index, reason }),
      failSub: (sub, reason) => post({ type: 'sub-failed', lane: index, sub, reason }),
      failWidget: (widget, reason) => post({ type: 'widget-failed', lane: index, widget, reason }),
      ...side.stretch ? { stretch: (ms: number) => side.stretch!(ms, rate) } : {},
    });
    port.onmessage = ({ data }: MessageEvent<FromPull>) => {
      if (data.kind === 'failure') { post({ type: 'failure', lane: index, reason: data.reason }); return; }
      try { made.receive(data.kind, data.bytes, data.kind === 'follow' ? data.resets : undefined); }
      catch (error) { post({ type: 'failure', lane: index, reason: error instanceof Error ? error.message : String(error) }); }
    };
    return made;
  };
  return m => {
    if (m.type === 'init') { rate = m.rate; lane = start(m.lane, m.lanes, m.port); return; }
    if (m.type === 'options') { rate = m.rate; return; }
    if (!lane) return;
    switch (m.type) {
      case 'follow': lane.follow(m.sub, m.channel, m.product); break;
      case 'route': lane.bindRoute(m.sub, m.route, m.native, m.seek, m.offset); break;
      case 'unfollow': lane.unfollow(m.sub); break;
      case 'mount': lane.mount(m.widget, m.subs, m.view, m.model); break;
      case 'unmount': lane.unmount(m.widget); break;
      case 'bind': lane.bind(m.widget, m.binding, m.model, m.retry); break;
      case 'exact': lane.exact(m.sub, m.hour, m.data, m.refused); break;
      case 'stream': lane.stream(m.route, m.seek, m.target); break;
      case 'fail': lane.fail(m.route); break;
      case 'pointer': lane.pointer(m.widget, m.surface, m.point); break;
      case 'frame': lane.frame(m); break;
    }
  };
}
