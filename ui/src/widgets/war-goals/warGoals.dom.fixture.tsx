import { createRoot } from 'react-dom/client';
import WarGoalsWidget from './WarGoalsWidget';
import { PlaybackController } from '../../dashboard/playback.ts';
import type { WidgetProps } from '../../dashboard/definitions';

const native = window.requestAnimationFrame.bind(window);
const frame = () => new Promise<void>(resolve => native(() => resolve()));

const HOURS = 10_000;
const track = { id: 'main', axisOffset: 0, selectedTag: 'GER', source: { id: 'source', revision: '1', metadata: { countries: [] } },
  branch: { id: '1', firstHour: 0, lastHour: HOURS - 1, gaps: [] } };
const follows: unknown[] = [];
const client = {
  available: true,
  product: async (request: { sourceId: string; revision: string; product: string; params: Record<string, unknown> }) => {
    if (request.product === 'follow') follows.push((request.params as { query?: { panel?: string } }).query?.panel);
    const data = request.product === 'follow' ? { available: false, reason: 'not pushed in this fixture' } : { available: true };
    return { sourceId: request.sourceId, revision: request.revision, product: request.product, data };
  },
  openFollow: async () => {},
  setMapViews: async () => {}, onMapStates: async () => () => {}, releaseSource: async () => {}, openSource: async () => { throw new Error('none'); },
};

export const fixture = {
  async cold() {
    const clock = new PlaybackController();
    clock.setBounds({ firstHour: 0, lastHour: HOURS - 1 });
    clock.seek(500);
    const host = document.body.appendChild(document.createElement('div'));
    host.style.cssText = 'width: 480px; height: 320px; display: flex;';
    const runtime = { tracks: [track], clock, client, addWidgetBelow() {}, campaigns: [], recordings: [], requestSeek() {} };
    const widget = { id: 'goals', kind: 'war-goals', trackIds: ['main'], countries: { kind: 'tags', tags: ['GER'] }, time: { mode: 'follow' }, options: {} };
    createRoot(host).render(<WarGoalsWidget {...{ widget, runtime, active: true, onDiagnostics() {} } as unknown as WidgetProps} />);
    for (let at = 0; at < 60 && !follows.length; at++) await frame();
    return { follows, text: host.textContent };
  },
};
(window as unknown as { warGoals: typeof fixture }).warGoals = fixture;
