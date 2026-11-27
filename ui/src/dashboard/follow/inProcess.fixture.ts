import { serialize } from 'node:v8';
import { laneDispatch } from '../../draw/laneDispatch.ts';
import { fieldsView } from '../../draw/views/fields.ts';
import { drawnViews } from '../../draw/views/drawn.ts';
import { PlaybackController } from '../playback.ts';
import { createPullDispatcher, type PullDispatcher } from '../pull.ts';
import { Pipeline, pipelineFor, subscribeClockPipeline, type LaneWorker, type PipelineHost } from './pipeline.ts';
import { charge, packet, resetStamps, splitForLanes } from './split.ts';
import type { DashboardClient, DashboardSource } from '../client';
import type { FromLane, FromPull, ToLane } from '../../draw/laneMessages';
import type { LaneView } from '../../draw/lane';

export const NATIVE = 3;

export interface Posted { readonly lane: number; readonly type: ToLane['type']; readonly bytes: number; readonly message: ToLane }

interface Held { hold: ((message: FromLane) => boolean) | null; queue: (() => void)[] }

function lane(views: (kind: string) => LaneView | undefined, onClose?: () => void, posted?: Posted[], held?: Held): LaneWorker & { close(): void } {
  const fields = fieldsView(() => undefined);
  let closed = false;
  let laneNumber = -1;
  let lanePort: MessagePort | null = null;
  const worker: LaneWorker & { close(): void } = {
    onmessage: null,
    postMessage(message: ToLane, transfer: Transferable[] = []) {
      if (closed) return;
      if (message.type === 'init') laneNumber = message.lane;
      else if (posted) posted.push({ lane: laneNumber, type: message.type, bytes: serialize(message).byteLength, message });
      const copy = structuredClone(message, { transfer: transfer as Transferable[] });
      if (copy.type === 'init') {
        lanePort = copy.port;
        (copy.port as MessagePort & { unref?(): void }).unref?.();
      }
      setImmediate(() => { if (!closed) dispatch(copy); });
    },
    close() {
      if (closed) return;
      closed = true;
      worker.onmessage = null;
      lanePort?.close();
      lanePort = null;
      onClose?.();
    },
    terminate() {
      worker.close();
    },
  };
  const dispatch = laneDispatch((message: FromLane) => {
    if (closed) return;
    const copy: FromLane = message.type === 'presentation'
      ? { ...structuredClone({ ...message, entries: [] }), entries: message.entries.map(entry => ({ ...structuredClone({ ...entry, surfaces: [] }), surfaces: entry.surfaces })) }
      : structuredClone(message);
    const deliver = () => { if (!closed) worker.onmessage?.({ data: copy } as MessageEvent<FromLane>); };
    setImmediate(() => { if (held?.hold?.(copy)) held.queue.push(deliver); else deliver(); });
  }, {
    views: (kind, wake) => views(kind) ?? (kind === 'fields' ? fields : drawnViews(kind, wake) ?? fields),
    crop: async sources => sources.map(() => ({ width: 1, height: 1, close() {} }) as unknown as ImageBitmap),
    later: run => setImmediate(() => { if (!closed) run(); }),
  });
  return worker;
}

export function inProcess({ lanes = 1, views = () => undefined, follow, followClock, gate, session, open }: { lanes?: number; views?: (kind: string) => LaneView | undefined;
  follow?: (id: number, query: Record<string, unknown>) => { available: boolean; reason?: string; route?: number };
  followClock?: (params: Record<string, unknown>) => { available: boolean; reason?: string };
  gate?: (product: string) => Promise<void> | undefined; session?: string;
  open?: (opened: readonly MessagePort[]) => Promise<string | undefined>; } = {}) {
  let ports: readonly MessagePort[] = [];
  let closed = false;
  const asked: { product: string; params: Record<string, unknown> }[] = [];
  const followers = new Set<unknown>();
  const takePorts = (opened: readonly MessagePort[]) => {
    const owned = structuredClone([...opened], { transfer: [...opened] });
    for (const port of owned) (port as MessagePort & { unref?(): void }).unref?.();
    return owned;
  };
  const client = {
    available: true,
    openFollow: (opened: readonly MessagePort[]) => {
      const owned = takePorts(opened);
      const replace = () => {
        for (const port of ports) port.close();
        ports = owned;
      };
      if (open) return open(owned).then(id => { replace(); return id; }, error => { for (const port of owned) port.close(); throw error; });
      replace();
      return Promise.resolve(session);
    },
    product: async (request: { product: string; params: Record<string, unknown> }) => {
      asked.push(request);
      await gate?.(request.product);
      if (request.product === 'follow') {
        const told = follow?.(request.params.id as number, request.params.query as Record<string, unknown>) ?? { available: true };
        if (told.available) followers.add(request.params.id);
        return { data: told.available ? { available: true, route: told.route ?? NATIVE } : told };
      }
      if (request.product === 'unfollow') followers.delete(request.params.id);
      if (request.product === 'follow-clock') {
        const told = followClock ? followClock(request.params) : { available: followers.size > 0 };
        return { data: told };
      }
      return { data: { available: true } };
    },
  } as unknown as DashboardClient;
  const clock = new PlaybackController();
  clock.setBounds({ firstHour: 0, lastHour: 100_000 });
  const channels = new Map<MessagePort, boolean>();
  const posted: Posted[] = [];
  const held: Held = { hold: null, queue: [] };
  const holdBack = (hold: (message: FromLane) => boolean) => { held.hold = hold; };
  const release = () => {
    held.hold = null;
    for (const deliver of held.queue.splice(0)) deliver();
  };
  const workers = Array.from({ length: lanes }, () => lane(views, undefined, posted, held));
  const host: PipelineHost = { lanes: workers, open: list => client.openFollow!(list),
    channel: () => {
      const made = new MessageChannel(), shut = made.port2.close.bind(made.port2);
      made.port2.close = () => { channels.set(made.port2, true); shut(); };
      channels.set(made.port2, false);
      return made;
    }, frameRate: () => 60, rate: () => 1,
    close: () => { for (const worker of workers) worker.close(); } };
  const pipeline = new Pipeline(clock, client, host);
  const stamp = resetStamps();
  const send = (bytes: Uint8Array) => {
    const resets = stamp(bytes), split = splitForLanes(bytes, lanes);
    const target = ports;
    setImmediate(() => {
      if (closed || ports !== target) return;
      split.lanes.forEach((part, index) => { if (part) target[index]?.postMessage(packet(split.kind, part, resets)); });
    });
  };
  const source = { id: 's', revision: 'r', path: 'p', metadata: {} } as unknown as DashboardSource;
  const settle = async (done: () => boolean, rounds = 200) => {
    for (let at = 0; at < rounds && !done(); at++) await new Promise(resolve => setTimeout(resolve, 2));
    return done();
  };
  const fail = (reason: string) => {
    const target = ports;
    setImmediate(() => {
      if (closed || ports !== target) return;
      for (const port of target) port.postMessage({ kind: 'failure', reason } satisfies FromPull);
    });
  };
  return { pipeline, clock, client, send, fail, source, asked, settle, channels, posted, holdBack, release, close: () => { closed = true; pipeline.close(); for (const port of ports) port.close(); ports = []; } };
}

export function managedInProcess({ lanes = 1, views = () => undefined, follow, followClock, gate, open }: {
  lanes?: number;
  views?: (kind: string) => LaneView | undefined;
  follow?: (id: number, query: Record<string, unknown>, session?: string) => { available: boolean; reason?: string; route?: number };
  followClock?: (params: Record<string, unknown>) => { available: boolean; reason?: string };
  gate?: (product: string, params: Record<string, unknown>) => Promise<void> | undefined;
  open?: (opened: readonly MessagePort[]) => Promise<string | undefined>;
} = {}) {
  let ports: readonly MessagePort[] = [];
  let dispatcher: PullDispatcher | null = null;
  let closed = false;
  let stamp = resetStamps();
  let openCalls = 0;
  let activeReaders = 0;
  let activeLanes = 0;
  let activeOwnedPorts = 0;
  let clockListeners = 0;
  let chargedBytes = 0;
  let creditedBytes = 0;
  const followers = new Set<string>();
  const waiters = new Map<string, (() => void)[]>();
  const notify = (event: string) => {
    const list = waiters.get(event);
    if (!list?.length) return;
    waiters.delete(event);
    for (const resolve of list) resolve();
  };
  const nextEvent = (event: string) => new Promise<void>(resolve => {
    let list = waiters.get(event);
    if (!list) waiters.set(event, list = []);
    list.push(resolve);
  });
  const client = {
    available: true,
    openFollow: async (opened: readonly MessagePort[]) => {
      openCalls++;
      const owned = structuredClone([...opened], { transfer: [...opened] });
      for (const port of owned) (port as MessagePort & { unref?(): void }).unref?.();
      activeReaders++;
      activeOwnedPorts += owned.length;
      let readerClosed = false;
      const madeDispatcher = createPullDispatcher(owned, bytes => {
        creditedBytes += bytes;
        notify('credit');
      });
      const closeReader = () => {
        if (readerClosed) return;
        readerClosed = true;
        activeReaders--;
        for (const port of owned) { port.onmessage = null; port.close(); activeOwnedPorts--; }
        if (ports === owned) { ports = []; dispatcher = null; }
      };
      try {
        const session = (open ? await open(owned) : undefined) ?? `session-${openCalls}`;
        ports = owned;
        dispatcher = madeDispatcher;
        stamp = resetStamps();
        notify('open');
        return { session, close: closeReader, retireLane: (laneIndex: number) => madeDispatcher.retireLane(laneIndex) };
      } catch (error) {
        closeReader();
        notify('open');
        throw error;
      }
    },
    product: async (request: { product: string; params: Record<string, unknown> }) => {
      await gate?.(request.product, request.params);
      const session = String(request.params.session ?? '');
      if (request.product === 'follow') {
        try {
          const told = follow?.(request.params.id as number, request.params.query as Record<string, unknown>, session) ?? { available: true };
          if (told.available) followers.add(`${session}:${request.params.id}`);
          return { data: told.available ? { available: true, route: told.route ?? NATIVE } : told };
        } finally {
          queueMicrotask(() => notify('follow'));
        }
      }
      if (request.product === 'unfollow') {
        followers.delete(`${session}:${request.params.id}`);
        queueMicrotask(() => notify('unfollow'));
      }
      if (request.product === 'follow-clock') {
        try {
          const told = followClock ? followClock(request.params) : { available: followers.size > 0 };
          return { data: told };
        } finally {
          queueMicrotask(() => notify('follow-clock'));
        }
      }
      return { data: { available: true } };
    },
  } as unknown as DashboardClient;
  const clock = new PlaybackController();
  clock.setBounds({ firstHour: 0, lastHour: 100_000 });
  const rawSub = clock.subscribe.bind(clock);
  clock.subscribe = listener => {
    clockListeners++;
    let done = false;
    const stop = rawSub(listener);
    return () => { if (!done) { done = true; clockListeners--; stop(); } };
  };
  const rawPub = clock.onPublish.bind(clock);
  clock.onPublish = (listener, first) => {
    clockListeners++;
    let done = false;
    const stop = rawPub(listener, first);
    return () => { if (!done) { done = true; clockListeners--; stop(); } };
  };
  const makeHost = (): PipelineHost => {
    const workers = Array.from({ length: lanes }, () => {
      activeLanes++;
      return lane(views, () => { activeLanes--; });
    });
    return {
      lanes: workers,
      open: list => client.openFollow!(list),
      channel: () => new MessageChannel(),
      frameRate: () => 60,
      rate: () => 1,
      close: () => { for (const worker of workers) worker.close(); },
    };
  };
  let current = pipelineFor(clock, client, makeHost);
  const stopSync = subscribeClockPipeline(clock, () => {
    current = pipelineFor(clock, client, makeHost);
    notify('restart');
  });
  const send = (bytes: Uint8Array) => {
    const resets = stamp(bytes);
    const target = dispatcher;
    const gen = openCalls;
    const copy = bytes.slice();
    setImmediate(() => {
      if (closed || !target || dispatcher !== target || openCalls !== gen) return;
      chargedBytes += charge(copy);
      target.dispatch(copy);
    });
    return resets;
  };
  const fail = (reason: string) => {
    const target = dispatcher;
    const gen = openCalls;
    setImmediate(() => {
      if (closed || !target || dispatcher !== target || openCalls !== gen) return;
      target.failAll(reason);
    });
  };
  const source = { id: 's', revision: 'r', path: 'p', metadata: {} } as unknown as DashboardSource;
  const waitView = (pipeline: Pipeline, widget: number, predicate: (view: ReturnType<Pipeline['view']>) => boolean) => {
    if (predicate(pipeline.view(widget))) return Promise.resolve();
    return new Promise<void>(resolve => {
      const stop = pipeline.onInstall(widget, () => {
        if (!predicate(pipeline.view(widget))) return;
        stop();
        resolve();
      });
    });
  };
  return {
    get pipeline() { return current; },
    clock, client, source, send, fail, nextEvent, waitView,
    credit: () => ({
      charged: chargedBytes,
      credited: creditedBytes,
      outstanding: dispatcher?.outstanding() ?? 0,
      laneOutstanding: (laneIndex: number) => dispatcher?.outstanding(laneIndex) ?? 0,
    }),
    counts: () => ({ openCalls, activeReaders, activeLanes, activeOwnedPorts, clockListeners }),
    close: () => { closed = true; stopSync(); current.close(); },
  };
}
