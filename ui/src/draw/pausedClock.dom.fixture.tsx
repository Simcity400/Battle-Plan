import { useRef } from 'react';
import { createRoot } from 'react-dom/client';
import { PlaybackController } from '../dashboard/playback';
import { useDrawnPanel } from '../dashboard/DrawnPanel';
import type { PanelFeed } from '../dashboard/panelFeed';
import DatedArsenal from '../charts/race/DatedArsenal';
import { readTokens } from '../charts/tokens';
import { arsenalNotice, arsenalStructure } from '../widgets/nuclear-arsenal/nuclearArsenalData';
import type { NuclearArsenalReply } from '../generated/dashboard';

const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
const HOURS = 100_000;
const reply: NuclearArsenalReply = { types: [{ ordinal: 0, token: 'nuke', name: 'Nuclear bomb' }],
  countries: [{ tag: 'GER', countryId: 1, total: 3, counts: [{ ordinal: 0, value: 3, ready: 3 }] }], missing: null, warnings: [] };
const date = { since: 0, through: HOURS - 1 };
const feed: PanelFeed = { view: 'nuclear-arsenal', offset: 0, structure: arsenalStructure(reply), reply, date };
const clock = new PlaybackController();
clock.setBounds({ firstHour: 0, lastHour: HOURS - 1 });
clock.seek(50_000);

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
      model={{ offset: 0, reply, date, top: 10, tags: ['GER'], split: false, tokens: readTokens(), slots: 5, overlay: false,
        colours: new Map([['GER', '#808080']]), looks: new Map([['GER', { name: 'Germany', color: '#808080' }]]) }} />
  </div>;
}

function shown() {
  const ink = document.querySelector<HTMLCanvasElement>('#panel canvas.draw-panel-ink');
  const race = document.querySelector<HTMLCanvasElement>('#race canvas');
  return { field: document.querySelector<HTMLElement>('#panel [data-paint="empty"]')?.dataset.rowInk ?? null,
    panel: !!ink && !ink.hidden, race: !!race && race.style.visibility !== 'hidden', hour: clock.getHour() };
}
const painted = () => { const now = shown(); return now.field === 'worker' && now.panel && now.race; };

async function settled() {
  for (let at = 0; at < 600 && !painted(); at++) await frame();
  return shown();
}

async function watch(change: () => void, frames = 30) {
  change();
  const seen = [shown()];
  for (let at = 0; at < frames; at++) { await frame(); seen.push(shown()); }
  return seen;
}

export const fixture = {
  async mount() {
    createRoot(document.body.appendChild(document.createElement('div'))).render(<><Panel /><Race /></>);
    return settled();
  },
  speed: (hoursPerSecond: number) => watch(() => clock.setSpeed(hoursPerSecond)),
  async playThenPause(frames: number) {
    clock.setPlaying(true);
    for (let at = 0; at < frames; at++) await frame();
    clock.setPlaying(false);
    return settled();
  },
  settled,
};
(window as unknown as { pausedClock: typeof fixture }).pausedClock = fixture;
