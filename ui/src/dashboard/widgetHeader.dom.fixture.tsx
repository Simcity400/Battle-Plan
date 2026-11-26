import { useState } from 'react';
import { flushSync } from 'react-dom';
import { createRoot, type Root } from 'react-dom/client';
import type { DashboardSource } from './client';
import type { WidgetDefinition, WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import WidgetHeader from './WidgetHeader';
import '../styles.css';
import './dashboard.css';

const source = { id: 'header', revision: 'r1', path: 'header', metadata: { branches: [{ id: '1', firstHour: 0, lastHour: 100_000 }] } } as unknown as DashboardSource;
const track = { id: 't1', source, branch: { id: '1', firstHour: 0, lastHour: 100_000 }, axisOffset: 0 };
let playing = false;
const clock = {
  getHour: () => 50_000,
  getSnapshot: () => ({ playing, hoursPerSecond: 1440 }),
  subscribe: () => () => {},
  subscribeFrame: () => () => {},
};
const runtime = { clock, tracks: [track] } as unknown as WidgetRuntime;
const definition = { kind: 'peace-deals', title: 'Peace deals', icon: 'grid', countrySelection: 'settings', timeModes: ['follow', 'campaign', 'pin'] } as unknown as WidgetDefinition;
const widgetIn = (time: WidgetSpec['time']) => ({ id: 'w1', kind: 'peace-deals', title: 'Peace deals', trackIds: ['t1'], time, countries: { kind: 'all' }, options: {} }) as unknown as WidgetSpec;

let setTime: (time: WidgetSpec['time']) => void = () => {};
let setWidth: (width: number) => void = () => {};

function Header() {
  const [time, set] = useState<WidgetSpec['time']>({ mode: 'follow' });
  const [width, resize] = useState(900);
  const [slot, setSlot] = useState<HTMLSpanElement | null>(null);
  setTime = next => flushSync(() => set(next));
  setWidth = next => flushSync(() => resize(next));
  const widget = widgetIn(time);
  return <article className="dash-widget" style={{ width: `${width}px`, height: '80px' }}>
    <WidgetHeader widget={widget} definition={definition} effective={widget} campaignTrack={track as never} runtime={runtime} activeSwitch={null}
      summarySlotRef={element => { if (element && !element.firstChild) element.append('15 peace deals'); setSlot(element); }}
      countrySlotRef={() => {}} headerSlotElement={slot} onApply={next => setTime(next.time)} />
  </article>;
}

const frames = (count: number) => new Promise<void>(resolve => {
  const step = (left: number) => requestAnimationFrame(() => (left <= 1 ? resolve() : step(left - 1)));
  step(count);
});

function shown() {
  const summary = document.querySelector<HTMLElement>('.dash-widget-summary')!;
  const chip = document.querySelector<HTMLElement>('.dash-time-chip')!;
  return { count: !summary.hidden, compact: chip.classList.contains('is-compact'), label: chip.querySelector('.dash-time-label')?.textContent ?? null };
}

let root: Root | null = null;

const api = {
  async mount() {
    root = createRoot(document.body.appendChild(document.createElement('div')));
    flushSync(() => root!.render(<Header />));
    await frames(10);
    let fits = 900, drops = 100;
    while (fits - drops > 1) {
      const middle = Math.floor((fits + drops) / 2);
      setWidth(middle);
      await frames(4);
      if (shown().count) fits = middle; else drops = middle;
    }
    setWidth(fits - 2);
    await frames(10);
    return shown();
  },
  async switchWhilePlaying(time: WidgetSpec['time']) {
    playing = true;
    setTime(time);
    await frames(10);
    return shown();
  },
};

(window as unknown as { widgetHeader: typeof api }).widgetHeader = api;
