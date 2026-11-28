import { flushSync } from 'react-dom';
import { createRoot } from 'react-dom/client';
import type { DashboardClient, DashboardSource } from '../dashboard/client.ts';
import type { WidgetRuntime } from '../dashboard/definitions.ts';
import { datedPanel } from '../dashboard/follow/messages.fixture.ts';
import { pipelineFor, type LaneWorker } from '../dashboard/follow/pipeline.ts';
import { SharedSurfaceContext, SharedSurfaceHost } from '../dashboard/follow/sharedSurface.ts';
import { PlaybackController } from '../dashboard/playback.ts';
import { laneDispatch } from '../draw/laneDispatch.ts';
import type { FromLane, ToLane } from '../draw/laneMessages.ts';
import { FrameEngine } from '../engine/frame.ts';
import { armySizeRaceProjection } from '../widgets/army-size/armySizeRaceProjection.ts';
import { barsHostAdapter } from './bars/hostAdapter.ts';
import { barsLane } from './bars/laneView.ts';
import FamilyHost from './host/FamilyHost.tsx';
import { createLaneView } from './laneRuntime.ts';
import '../styles.css';
import '../dashboard/dashboard.css';

const until = async (ready: () => boolean) => {
  for (let i = 0; i < 80; i++) {
    if (ready()) return true;
    await new Promise<void>(done => requestAnimationFrame(() => done()));
  }
  return false;
};

async function checkHostedRefusal() {
  let port: MessagePort | undefined;
  let read: { id: number; route: number } | undefined;
  const client = {
    available: true,
    openFollow: async (ports: readonly MessagePort[]) => { port = ports[0]; },
    product: async (request: { product: string; params: { id: number; route: number } }) => {
      if (request.product === 'follow') read = request.params;
      return { data: { available: true, route: request.params.route } };
    },
  } as unknown as DashboardClient;
  const branch = { id: '1', firstHour: 100, lastHour: 110 };
  const source = { id: 'motion-host', revision: 'fixture', path: 'motion-host', metadata: { branches: [branch] } } as unknown as DashboardSource;
  const frames = new FrameEngine({ request() {} });
  const clock = new PlaybackController(frames, 'motion-host-fixture');
  clock.setBounds(branch);
  const runtime = { client, clock, tracks: [{ id: 't', source, branch, axisOffset: 0 }] } as unknown as WidgetRuntime;
  let now = performance.now();
  const labels = new Set<string>();
  const worker: LaneWorker = {
    onmessage: null,
    postMessage(message: ToLane) {
      if (message.type === 'lane-surface') {
        const ctx = message.canvas.getContext('2d')!;
        const fillText = ctx.fillText.bind(ctx);
        ctx.fillText = (label, x, y, width) => {
          labels.add(label);
          if (width === undefined) fillText(label, x, y);
          else fillText(label, x, y, width);
        };
      }
      queueMicrotask(() => dispatch(message));
    },
  };
  const dispatch = laneDispatch(message => queueMicrotask(() => worker.onmessage?.({ data: message } as MessageEvent<FromLane>)), {
    views: (_kind, wake) => createLaneView(barsLane, wake),
    crop: async canvases => Promise.all(canvases.map(canvas => createImageBitmap(canvas as OffscreenCanvas))),
    later: draw => requestAnimationFrame(draw),
    clock: () => now,
  });
  const pipeline = pipelineFor(clock, client, () => ({
    lanes: [worker], open: ports => client.openFollow!(ports), channel: () => new MessageChannel(), frameRate: () => 60, rate: () => 1,
  }));
  const viewport = document.createElement('div');
  viewport.style.cssText = 'position:relative;overflow:auto;width:480px;height:240px';
  document.body.append(viewport);
  const mount = document.createElement('div');
  mount.style.cssText = 'position:relative;width:480px;height:240px';
  viewport.append(mount);
  const shared = new SharedSurfaceHost(pipeline, viewport);
  const root = createRoot(mount);
  try {
    flushSync(() => root.render(<SharedSurfaceContext.Provider value={shared}>
      <FamilyHost adapter={barsHostAdapter} declaration={armySizeRaceProjection} view="race"
        options={{ surface: '#16181d', top: 10, group: false, split: false, measure: 'divisions', countFor: 'owner' }}
        label="Army size" subject="armies"
        widget={{ id: 'rank', kind: 'army-size', configVersion: 1, title: 'Army size', trackIds: ['t'], countries: { kind: 'all' }, time: { mode: 'follow' } }}
        runtime={runtime} active subscriptions={[{
          name: 'main', source, branch: '1', offset: 0, query: { panel: 'series-rank', branchId: '1' },
          channel: 'panel', product: 'series-rank',
        }]} offset={0} pin={null} />
    </SharedSurfaceContext.Provider>));
    if (!await until(() => !!read && pipeline.status(read.id).status === 'followed')) throw new Error('Rank subscription did not open');
    const send = (hour: number, value: number | null) => port!.postMessage({
      kind: 'panels',
      bytes: datedPanel(read!.id, hour, {
        branchId: '1', hour, from: hour, until: hour, measures: [{ name: 'ownerDivisions', decimals: 0 }],
        refused: value === null ? 'The ranking is incomplete' : null,
        rows: value === null ? [] : [{ country: 'GER', readings: [{
          country: 'GER', hour, drop: null, missing: null,
          run: { valueRaw: String(value), firstHour: hour, lastHour: hour, slot: 1, countryId: 1, startsSegment: false, segmentFirstHour: 0, firstExact: true, lastExact: true },
        }] }],
      }, clock.getSnapshot().seekSequence, read!.route),
    });
    const overlay = () => mount.querySelector('.fh-overlay')?.textContent ?? null;
    const sharedActive = () => mount.querySelector('.fh-frame')?.getAttribute('data-shared-surface') === '1';
    const placementOf = () => Number(mount.querySelector<HTMLCanvasElement>('canvas.fh-canvas')?.dataset.placement);
    send(100, 37);
    if (!await until(() => sharedActive() && labels.has('37') && pipeline.view(placementOf())?.through === 100)) {
      throw new Error('Initial rank did not install on the shared canvas');
    }
    const placement = placementOf();
    send(101, null);
    clock.setPlaying(true);
    const anchor = clock.composedAt();
    const tick = (offset: number) => { now = anchor + offset; frames.tick(now); };
    const refusal = await until(() => {
      tick(1500);
      return pipeline.view(placement)?.through === 101 && sharedActive() && overlay()?.includes('The ranking is incomplete') === true;
    });
    if (!refusal) throw new Error('Rank refusal did not install while keeping the shared canvas active');
    labels.clear();
    const held = await until(() => {
      tick(2500);
      return overlay() === null && sharedActive() && labels.has('37');
    });
    if (!held) throw new Error('Expired rank refusal did not reveal the held reading');
    const gap = pipeline.view(placement);
    labels.clear();
    mount.querySelector<HTMLElement>('.fh-body')!.focus();
    const rebound = await until(() => {
      tick(2500);
      const view = pipeline.view(placement);
      return overlay() === null && labels.has('37') && view?.through === 100
        && (view.installedBinding ?? 0) > (gap?.installedBinding ?? 0);
    });
    send(103, 83);
    labels.clear();
    const resumed = await until(() => {
      tick(3500);
      return overlay() === null && labels.has('83') && pipeline.view(placement)?.through === 103;
    });
    return { through: gap?.through, painted: gap?.painted, rebound, resumed };
  } finally {
    clock.stop();
    flushSync(() => root.unmount());
    shared.dispose();
    pipeline.close();
    viewport.remove();
  }
}

(window as unknown as { checkHostedRefusal: typeof checkHostedRefusal }).checkHostedRefusal = checkHostedRefusal;
