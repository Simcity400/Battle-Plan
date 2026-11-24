import { Lane, type LaneHost, type LaneView } from './lane.ts';
import type { FromLane, FromPull, ToLane, ToPull } from './laneMessages';

export interface WakeChannel {
  readonly port1: { onmessage: ((ev: MessageEvent) => unknown) | null };
  readonly port2: { postMessage(message: unknown): void };
}

export function macrotaskSchedule(
  createChannel: (() => WakeChannel) | null = typeof MessageChannel === 'function' ? () => new MessageChannel() : null,
): ((run: () => void) => void) | undefined {
  if (!createChannel) return undefined;
  const wake = createChannel();
  let current: { token: number; run: () => void } | null = null;
  let nextToken = 1;
  wake.port1.onmessage = (event: MessageEvent) => {
    const active = current;
    if (!active || event.data !== active.token) return;
    current = null;
    active.run();
  };
  return run => {
    const token = nextToken++;
    current = { token, run };
    wake.port2.postMessage(token);
  };
}

export function cropAll<S, B extends { close(): void }>(sources: readonly S[], make: (source: S) => Promise<B>): Promise<B[]> {
  if (!sources.length) return Promise.resolve([]);
  return new Promise<B[]>((resolve, reject) => {
    const out = new Array<B | undefined>(sources.length);
    let remaining = sources.length;
    let failed = false;
    const failWith = (error: unknown) => {
      if (failed) return;
      failed = true;
      for (let i = 0; i < out.length; i++) {
        out[i]?.close();
        out[i] = undefined;
      }
      reject(error);
    };
    sources.forEach((source, index) => {
      if (failed) return;
      let pending: Promise<B>;
      try { pending = make(source); }
      catch (error) { failWith(error); return; }
      pending.then(
        bitmap => {
          if (failed) {
            bitmap.close();
            return;
          }
          out[index] = bitmap;
          if (--remaining === 0) resolve(out as B[]);
        },
        failWith,
      );
    });
  });
}

export interface LaneSide {
  views(kind: string, wake: (widget: number) => void): LaneView;
  crop: LaneHost<ImageBitmap>['crop'];
  later(run: () => void): void;
  schedule?: LaneHost<ImageBitmap>['schedule'];
  clock?: LaneHost<ImageBitmap>['clock'];
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
      ...side.clock ? { clock: side.clock } : {},
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
