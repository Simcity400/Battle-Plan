import { laneDispatch } from '../../draw/laneDispatch.ts';
import { fieldsView } from '../../draw/views/fields.ts';
import { drawnViews } from '../../draw/views/drawn.ts';
import { PlaybackController } from '../playback.ts';
import { Pipeline, type LaneWorker, type PipelineHost } from './pipeline.ts';
import { packet, resetStamps, splitForLanes } from './split.ts';
import type { DashboardClient, DashboardSource } from '../client';
import type { FromLane, FromPull, ToLane } from '../../draw/laneMessages';
import type { LaneView } from '../../draw/lane';

export const NATIVE = 3;

function lane(views: (kind: string) => LaneView | undefined): LaneWorker {
  const fields = fieldsView(() => undefined);
  const worker: LaneWorker = {
    onmessage: null,
    postMessage(message: ToLane, transfer: Transferable[] = []) {
      const copy = structuredClone(message, { transfer: transfer as Transferable[] });
      if (copy.type === 'init') (copy.port as MessagePort & { unref?(): void }).unref?.();
      setImmediate(() => dispatch(copy));
    },
  };
  const dispatch = laneDispatch((message: FromLane) => {
    const copy: FromLane = message.type === 'presentation'
      ? { ...structuredClone({ ...message, entries: [] }), entries: message.entries.map(entry => ({ ...structuredClone({ ...entry, surfaces: [] }), surfaces: entry.surfaces })) }
      : structuredClone(message);
    setImmediate(() => worker.onmessage?.({ data: copy } as MessageEvent<FromLane>));
  }, {
    views: (kind, wake) => views(kind) ?? (kind === 'fields' ? fields : drawnViews(kind, wake) ?? fields),
    crop: async sources => sources.map(() => ({ width: 1, height: 1, close() {} }) as unknown as ImageBitmap),
    later: run => setTimeout(run, 16),
  });
  return worker;
}

export function inProcess({ lanes = 1, views = () => undefined, follow, followClock, gate, session, open }: { lanes?: number; views?: (kind: string) => LaneView | undefined;
  follow?: (id: number, query: Record<string, unknown>) => { available: boolean; reason?: string; route?: number };
  followClock?: (params: Record<string, unknown>) => { available: boolean; reason?: string };
  gate?: (product: string) => Promise<void> | undefined; session?: string;
  open?: (opened: readonly MessagePort[]) => Promise<string | undefined>; } = {}) {
  let ports: readonly MessagePort[] = [];
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
  const host: PipelineHost = { lanes: Array.from({ length: lanes }, () => lane(views)), open: list => client.openFollow!(list),
    channel: () => {
      const made = new MessageChannel(), shut = made.port2.close.bind(made.port2);
      made.port2.close = () => { channels.set(made.port2, true); shut(); };
      channels.set(made.port2, false);
      return made;
    }, frameRate: () => 60, rate: () => 1 };
  const pipeline = new Pipeline(clock, client, host);
  const stamp = resetStamps();
  const send = (bytes: Uint8Array) => {
    const resets = stamp(bytes), split = splitForLanes(bytes, lanes);
    if (split.kind !== 'failure') split.lanes.forEach((part, index) => { if (part) ports[index]!.postMessage(packet(split.kind, part, resets)); });
  };
  const source = { id: 's', revision: 'r', path: 'p', metadata: {} } as unknown as DashboardSource;
  const settle = async (done: () => boolean, rounds = 200) => {
    for (let at = 0; at < rounds && !done(); at++) await new Promise(resolve => setTimeout(resolve, 2));
    return done();
  };
  const fail = (reason: string) => { for (const port of ports) port.postMessage({ kind: 'failure', reason } satisfies FromPull); };
  return { pipeline, clock, client, send, fail, source, asked, settle, channels, close: () => { pipeline.close(); for (const port of ports) port.close(); } };
}
