import { useRef } from 'react';
import { createRoot } from 'react-dom/client';
import { PlaybackController } from '../dashboard/playback';
import { useDrawnPanel } from '../dashboard/DrawnPanel';
import type { PanelFeed } from '../dashboard/panelFeed';
import { clockGeneration, onDraw } from '../dashboard/pulled';
import DatedArsenal from '../charts/race/DatedArsenal';
import { readTokens } from '../charts/tokens';
import { FrameEngine } from '../engine/frame';
import { arsenalNotice, arsenalStructure } from '../widgets/nuclear-arsenal/nuclearArsenalData';
import type { NuclearArsenalReply } from '../generated/dashboard';
import type { Present } from './notifications';
import type { FromWorker } from './kit';

const HOURS = 100_000;
const reply: NuclearArsenalReply = { types: [{ ordinal: 0, token: 'nuke', name: 'Nuclear bomb' }],
  countries: [{ tag: 'GER', countryId: 1, total: 3, counts: [{ ordinal: 0, value: 3, ready: 3 }] }], missing: null, warnings: [] };
const date = { since: 0, through: HOURS - 1 };
const feed: PanelFeed = { view: 'nuclear-arsenal', offset: 0, structure: arsenalStructure(reply), reply, date };
const clock = new PlaybackController(new FrameEngine({ request() {} }));
clock.setBounds({ firstHour: 0, lastHour: HOURS - 1 });
clock.seek(23);

let panelPresent: Present | undefined, racePresent: Extract<FromWorker, { type: 'racePresent' }> | undefined;
const waiting = new Set<() => void>();
onDraw(message => {
  if (message.type === 'present') panelPresent = message.surfaces.at(-1) ?? panelPresent;
  if (message.type === 'racePresent') racePresent = message;
  queueMicrotask(() => { for (const heard of [...waiting]) heard(); });
});

function Panel() {
  const root = useRef<HTMLDivElement>(null);
  useDrawnPanel(root, clock, feed, true);
  return <div id="panel" ref={root} style={{ width: '480px', height: '40px' }}>
    <span data-paint="empty">{arsenalNotice(reply, clock.getHour())}</span>
  </div>;
}

function Race() {
  return <div id="race" style={{ width: '480px', height: '160px', display: 'flex' }}>
    <DatedArsenal label="Nuclear arsenal" items={[]} slots={5} name={key => key} active clock={clock}
      model={{ offset: 0, reply, date, top: 10, tags: ['GER'], split: false, tokens: readTokens(), slots: 5, overlay: true,
        colours: new Map([['GER', '#808080']]), looks: new Map([['GER', { name: 'Germany', color: '#808080' }]]) }} />
  </div>;
}

function shown() {
  const ink = document.querySelector<HTMLCanvasElement>('#panel canvas.draw-panel-ink');
  const race = document.querySelector<HTMLCanvasElement>('#race canvas');
  return { field: document.querySelector<HTMLElement>('#panel [data-paint="empty"]')?.dataset.rowInk ?? null,
    panel: !!ink && !ink.hidden, race: !!race && race.style.visibility !== 'hidden', hour: clock.getHour(),
    panelHour: panelPresent?.readHour ?? null, raceHour: racePresent?.hour ?? null };
}
const painted = () => { const now = shown(); return now.field === 'worker' && now.panel && now.race; };
const current = () => {
  const generation = clockGeneration(clock), seek = clock.getSnapshot().seekSequence;
  return panelPresent?.generation === generation && panelPresent.seek === seek && racePresent?.generation === generation && racePresent.seek === seek;
};

function until(ready: () => boolean) {
  return new Promise<void>(resolve => {
    const heard = () => { if (ready()) { waiting.delete(heard); resolve(); } };
    waiting.add(heard); heard();
  });
}

async function watch(change: () => void) {
  change();
  const seen = [shown()];
  let watching = true;
  const sample = () => { if (!watching) return; seen.push(shown()); requestAnimationFrame(sample); };
  requestAnimationFrame(sample);
  await until(current);
  watching = false;
  seen.push(shown());
  return seen;
}

function at(time: number, change: () => void) {
  performance.now = () => time;
  try { change(); } finally { Reflect.deleteProperty(performance, 'now'); }
}

export const fixture = {
  async mount() {
    createRoot(document.body.appendChild(document.createElement('div'))).render(<><Panel /><Race /></>);
    await until(() => current() && painted());
    return shown();
  },
  speed: (hoursPerSecond: number) => watch(() => clock.setSpeed(hoursPerSecond)),
  seek: (hour: number) => watch(() => clock.seek(hour)),
  step: () => watch(() => clock.step(1)),
  async playFor(milliseconds: number) {
    const start = performance.now();
    at(start, () => clock.setPlaying(true));
    await until(current);
    at(start + milliseconds, () => clock.setPlaying(false));
    await until(() => current() && painted());
    return shown();
  },
};
(window as unknown as { pausedClock: typeof fixture }).pausedClock = fixture;
