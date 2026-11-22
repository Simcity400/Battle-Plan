import { useLayoutEffect, useRef } from 'react';
import { flushSync } from 'react-dom';
import { createRoot } from 'react-dom/client';
import { PlaybackController } from './playback.ts';
import { useDrawnPanel } from './DrawnPanel';
import { useRenderAt } from './headProduct.ts';
import { invalidateGeometry } from '../charts/geometry.ts';
import { engine } from '../engine/frame.ts';
import { arsenalNotice, arsenalStructure } from '../widgets/nuclear-arsenal/nuclearArsenalData.ts';
import type { NuclearArsenalReply } from '../generated/dashboard';
import type { FromWorker, ToWorker } from '../draw/kit';

const heard: FromWorker[] = [];
const held: { worker: Worker; message: ToWorker; transfer: Transferable[] }[] = [];
let holding = false;
class Watched extends window.Worker {
  constructor(url: string | URL, options?: WorkerOptions) {
    super(url, options);
    this.addEventListener('message', event => heard.push((event as MessageEvent<FromWorker>).data));
  }
  postMessage(message: unknown, transfer?: Transferable[] | StructuredSerializeOptions): void {
    if (holding && (message as ToWorker).type === 'surface') {
      held.push({ worker: this, message: message as ToWorker, transfer: Array.isArray(transfer) ? transfer : [] });
      return;
    }
    super.postMessage(message, transfer as Transferable[]);
  }
}
window.Worker = Watched;

const native = window.requestAnimationFrame.bind(window);
const frame = () => new Promise<void>(resolve => native(() => resolve()));
async function frames(count: number) { for (let at = 0; at < count; at++) await frame(); }
async function until(done: () => boolean, budget = 240) {
  for (let at = 0; at < budget && !done(); at++) await frame();
  return done();
}

const clock = new PlaybackController();
clock.setBounds({ firstHour: 0, lastHour: 100_000 });
const reply: NuclearArsenalReply = { missing: null, types: [], warnings: [], countries: [] };

function Arsenal({ fixed }: { fixed: number }) {
  const root = useRef<HTMLDivElement>(null);
  useDrawnPanel(root, clock, { view: 'nuclear-arsenal', offset: 0, structure: arsenalStructure(reply), fixed,
    date: { since: fixed, through: fixed }, reply }, true);
  return <div ref={root} style={{ width: '480px', padding: '8px', font: '16px sans-serif' }}>
    <p data-paint="empty" style={{ margin: 0 }}>{arsenalNotice(reply, fixed)}</p>
  </div>;
}

const reads: { text: string; frame: number | null; phase: string | null }[] = [];
function Due({ at }: { at: number }) {
  useRenderAt(at, clock);
  const box = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => invalidateGeometry(() => {
    reads.push({ text: box.current?.textContent ?? '', frame: engine.frame?.index ?? null, phase: engine.frame?.phase ?? null });
    return () => {};
  }));
  return <div ref={box} style={{ width: '200px' }}>{clock.getHour() >= at ? 'due' : 'waiting'}</div>;
}

const place = () => document.body.appendChild(document.createElement('div'));

export const fixture = {
  async rebind(first: number, second: number) {
    const host = place(), root = createRoot(host);
    const canvas = () => host.querySelector<HTMLCanvasElement>('canvas.draw-panel-ink');
    root.render(<Arsenal fixed={first} />);
    await frames(2);
    root.render(<Arsenal fixed={first} />);
    const shown = await until(() => !!canvas() && !canvas()!.hidden);
    const before = canvas()!;
    holding = true;
    flushSync(() => root.render(<Arsenal fixed={second} />));
    const hiddenAtOnce = before.hidden, sameCanvas = canvas() === before, canvases = host.querySelectorAll('canvas').length;
    await until(() => held.length > 0);
    const surface = held[0]?.message as Extract<ToWorker, { type: 'surface' }> | undefined;
    const presents = () => heard.flatMap(message => message.type === 'present' ? message.surfaces.filter(p => p.id === surface?.id) : []);
    heard.length = 0;
    clock.seek(clock.getSnapshot().position);
    await until(() => presents().some(p => p.owner < (surface?.owner ?? 0)));
    await frames(2);
    const oldOwnerPresents = presents().filter(p => p.owner < (surface?.owner ?? 0)).length, hiddenAfterOldOwner = before.hidden;
    const heldSurfaces = held.length;
    holding = false;
    for (const { worker, message, transfer } of held.splice(0)) worker.postMessage(message, transfer);
    const revealed = await until(() => !before.hidden);
    const newOwnerPresents = presents().filter(p => p.owner === surface?.owner).length;
    root.unmount(); host.remove();
    return { shown, hiddenAtOnce, sameCanvas, canvases, heldSurfaces, oldOwnerPresents, hiddenAfterOldOwner, revealed, newOwnerPresents,
      fixed: surface?.model.fixed ?? null };
  },
  async due(at: number) {
    const host = place(), root = createRoot(host);
    clock.seek(at - 2);
    root.render(<Due at={at} />);
    await frames(4);
    reads.length = 0;
    let dueFrame: number | null = null;
    const stop = clock.subscribeFrame(() => { if (dueFrame === null && clock.getHour() >= at) dueFrame = engine.frame?.index ?? null; });
    clock.setSpeed(24);
    clock.setPlaying(true);
    await until(() => reads.some(read => read.text === 'due'));
    clock.setPlaying(false);
    stop();
    await frames(2);
    root.unmount(); host.remove();
    return { dueFrame, reads: [...reads] };
  },
};
(window as unknown as { drawnPanel: typeof fixture }).drawnPanel = fixture;
