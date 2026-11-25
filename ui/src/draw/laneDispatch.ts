import { Lane, type Bitmapped, type LaneHost, type LaneView } from './lane.ts';
import type { FromLane, FromPull, ToLane, ToPull } from './laneMessages';
import { WORKER_PRESENTS } from '../proof/arm.ts';
import { Stage } from '../proof/stage.ts';

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

function macrotaskQueue(): (run: () => void) => void {
  if (typeof MessageChannel !== 'function') return run => setTimeout(run, 0);
  const channel = new MessageChannel(), queue: (() => void)[] = [];
  channel.port1.onmessage = () => { const run = queue.shift(); run?.(); };
  return run => { queue.push(run); channel.port2.postMessage(0); };
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

interface TransferableSurface<B> extends Bitmapped {
  transferToImageBitmap?(): B;
}

const canvasRestores = new WeakMap<object, () => void>();
const transferredCanvases = new WeakSet<object>();

export function markCanvasPainted(canvas: object, restore: () => void): void {
  canvasRestores.set(canvas, restore);
  transferredCanvases.delete(canvas);
}

const defaultFallback = (source: Bitmapped): Promise<ImageBitmap> => createImageBitmap(source as OffscreenCanvas);

export function cropSurfaces<B extends { close(): void } = ImageBitmap>(
  sources: readonly Bitmapped[],
  fallback: (source: Bitmapped) => Promise<B> = defaultFallback as unknown as (source: Bitmapped) => Promise<B>,
): Promise<B[]> {
  if (!sources.length) return Promise.resolve([]);
  const remaining = new Map<Bitmapped, number>();
  let allTransferable = true;
  for (let i = 0; i < sources.length; i++) {
    const source = sources[i]!;
    const count = (remaining.get(source) ?? 0) + 1;
    remaining.set(source, count);
    if (
      count > 1
      || !canvasRestores.has(source)
      || source.width <= 0
      || source.height <= 0
      || typeof (source as TransferableSurface<B>).transferToImageBitmap !== 'function'
    ) {
      allTransferable = false;
    }
  }
  if (allTransferable) {
    const out: B[] = [];
    try {
      for (let i = 0; i < sources.length; i++) {
        const source = sources[i] as TransferableSurface<B>;
        if (transferredCanvases.has(source)) {
          canvasRestores.get(source)!();
          transferredCanvases.delete(source);
        }
        const bitmap = source.transferToImageBitmap!();
        transferredCanvases.add(source);
        out.push(bitmap);
      }
      return Promise.resolve(out);
    } catch (error) {
      for (let i = 0; i < out.length; i++) out[i]!.close();
      return Promise.reject(error);
    }
  }
  return cropAll(sources, source => {
    const left = remaining.get(source)!;
    remaining.set(source, left - 1);
    const restore = canvasRestores.get(source);
    if (restore && transferredCanvases.has(source)) {
      restore();
      transferredCanvases.delete(source);
    }
    const surface = source as TransferableSurface<B>;
    if (restore && left === 1 && surface.width > 0 && surface.height > 0 && typeof surface.transferToImageBitmap === 'function') {
      const bitmap = surface.transferToImageBitmap();
      transferredCanvases.add(source);
      return Promise.resolve(bitmap);
    }
    return fallback(source);
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
  const defer = macrotaskQueue();
  const stretchLater = (ms: number) => { if (side.stretch && ms > 0) defer(() => side.stretch!(ms, rate)); };
  const stage = WORKER_PRESENTS ? new Stage(message => post(message), stretchLater) : null;
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
      present: (presentation, transfer) => stage ? stage.offer(presentation) : post({ type: 'presentation', ...presentation }, transfer),
      release: bytes => port.postMessage({ release: bytes } satisfies ToPull),
      hit: (widget, key, installedId, owner) => post({
        type: 'hit',
        widget,
        key,
        ...(installedId !== undefined ? { installedId } : {}),
        ...(owner !== undefined ? { owner } : {}),
      }),
      served: frame => post({ type: 'served', ...frame }),
      fail: reason => post({ type: 'failure', lane: index, reason }),
      failSub: (sub, reason) => post({ type: 'sub-failed', lane: index, sub, reason }),
      failWidget: (widget, reason, stamp) => post({ type: 'widget-failed', lane: index, widget, reason, ...stamp !== undefined ? { stamp } : {} }),
      ...side.stretch ? { stretch: stage ? stretchLater : (ms: number) => side.stretch!(ms, rate) } : {},
    });
    stage?.start(made, index);
    port.onmessage = ({ data }: MessageEvent<FromPull>) => {
      if (data.kind === 'failure') { post({ type: 'failure', lane: index, reason: data.reason }); return; }
      performance.mark(`bp-recv-${data.kind}-${data.bytes.byteLength}`);
      try { made.receive(data.kind, data.bytes, data.kind === 'follow' ? data.resets : undefined); }
      catch (error) { post({ type: 'failure', lane: index, reason: error instanceof Error ? error.message : String(error) }); }
    };
    return made;
  };
  return m => {
    if (m.type === 'init') { rate = m.rate; lane = start(m.lane, m.lanes, m.port); return; }
    if (m.type === 'options') { rate = m.rate; return; }
    if (m.type === 'canvas') { stage?.attach(m.widget, m.canvas); return; }
    if (m.type === 'clock') { stage?.clock(m); return; }
    if (!lane) return;
    switch (m.type) {
      case 'follow': lane.follow(m.sub, m.channel, m.product); break;
      case 'route': lane.bindRoute(m.sub, m.route, m.native, m.seek, m.offset); break;
      case 'unfollow': lane.unfollow(m.sub); break;
      case 'mount': lane.mount(m.widget, m.subs, m.view, m.model); break;
      case 'unmount': lane.unmount(m.widget); stage?.detach(m.widget); break;
      case 'bind': lane.bind(m.widget, m.binding, m.model, m.retry, m.remounted); break;
      case 'supply': lane.supply(m.widget, m); break;
      case 'payloads': lane.payloads(m); break;
      case 'exact': lane.exact(m.sub, m.hour, m.data, m.refused); break;
      case 'stream': lane.stream(m.route, m.seek, m.target); break;
      case 'fail': lane.fail(m.route); break;
      case 'pointer': lane.pointer(m.widget, m.surface, m.point, m.installed); break;
      case 'frame': if (stage) stage.exact(m); else lane.frame(m); break;
    }
  };
}
