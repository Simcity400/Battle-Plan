import '../styles.css';
import { createRef } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { PlaybackController } from '../dashboard/playback.ts';
import { MapViewsStore } from '../dashboard/geometry.ts';
import type { WidgetRuntime } from '../dashboard/definitions.ts';
import type { BoundTrack } from '../dashboard/model.ts';
import LandBattleWindow from '../widgets/land-battles/LandBattleWindow.tsx';
import NavalBattleWindow from '../widgets/naval-battles/NavalBattleWindow.tsx';

const pending = new Promise<never>(() => {});
const settle = async () => { for (let i = 0; i < 5; i++) await new Promise(resolve => requestAnimationFrame(resolve)); };
let root: Root | null = null;
let release: (() => void) | undefined;
let opened: Element | null = null;
const title = () => document.querySelector('.detail-window-header strong')?.textContent ?? '';
Object.assign(window, { battleTitle: {
  async mount(kind: 'land' | 'naval', missing = false) {
    root?.unmount();
    document.body.replaceChildren();
    release = undefined;
    const client = { available: true, nativeMaps: false, product: (request: { sourceId: string; revision: string; product: string; params: { battles: string[] } }) => {
      if (request.product !== 'battle-names') return pending;
      return new Promise(resolve => { release = () => resolve({ ...request, data: {
        titles: Object.fromEntries(request.params.battles.map(key => [key, missing
          ? { name: null, missing: 'The province name was withheld' }
          : { name: 'Second Battle of Copper Hollow', missing: null }])), warnings: [],
      } }); });
    } };
    const branch = { id: '1', firstHour: 100, lastHour: 109, gaps: [] };
    const source = { id: kind, revision: '1', path: 'title.battleplan', metadata: { branches: [branch], countries: [] } };
    const track = { id: 'main', source, branch, axisOffset: 0 } as unknown as BoundTrack;
    const clock = new PlaybackController({ every: () => ({ wake() {}, stop() {} }) } as never);
    clock.setBounds({ firstHour: 100, lastHour: 109 });
    clock.seek(103);
    const runtime = { client, clock, tracks: [track], maps: new MapViewsStore(client as never, clock), nativeHidden: true } as unknown as WidgetRuntime;
    const element = document.body.appendChild(document.createElement('div'));
    const anchor = createRef<HTMLDivElement>();
    anchor.current = element;
    const row = { key: kind === 'land' ? '1:7:101' : 'result:7', hour: kind === 'land' ? 101 : 98, province: 51, state: null, countries: [[], []], countriesHour: [101, 101],
      countriesUnknown: [false, false], result: 'ongoing', readHour: 103, detailId: 'result:7', detailHour: null,
      resultHour: null, lastLiveHour: null, endSeen: false, shipsSunk: [0, 0], convoysSunk: [0, 0], icLost: [0, 0],
      ships: [0, 0], convoys: [0, 0], casualties: [0, 0], divisions: [0, 0] };
    const props = { opened: { scope: kind, subject: { row, title: 'Loading name…', colors: [] }, hour: 103 }, track, runtime, anchor, onClose() {} };
    root = createRoot(element);
    root.render(kind === 'land' ? <LandBattleWindow {...props as unknown as Parameters<typeof LandBattleWindow>[0]} /> : <NavalBattleWindow {...props as unknown as Parameters<typeof NavalBattleWindow>[0]} />);
    await settle();
    opened = document.querySelector('.detail-window');
    return title();
  },
  async resolveName() {
    if (!release) throw new Error('The window never requested its battle title');
    release();
    await settle();
    return { title: title(), sameWindow: opened !== null && document.querySelector('.detail-window') === opened };
  },
  missingReason: () => document.querySelector('.detail-window-header [data-value-state=missing]')?.getAttribute('aria-label'),
} });
