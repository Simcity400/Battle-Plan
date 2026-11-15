/** What a value at the head asks of the service while the hour moves: the measure series' reads as today (the
 * head among them) and as a followed widget makes them (the head pushed, not read), and the follow connection's
 * requests while a clock plays. Mounted in a real browser so effects and reads run as in the app. */
import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import type { DashboardClient, DashboardSource, ProductRequest } from '../client';
import { PlaybackController } from '../playback';
import { useMeasureSeries } from '../../widgets/country-measure/measureSeries';
import { followTransport } from './connection.ts';

const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function settle() { for (let i = 0; i < 6; i++) await frame(); }

const asked: string[] = [];
const source = { id: 'lease', path: 'campaign.battleplan', revision: 'r1', metadata: {} } as unknown as DashboardSource;

function reply(request: ProductRequest) {
  const params = request.params as { reads?: { tag: string; hour: number }[]; tags?: string[]; family?: { measure: string } };
  const data = request.product === 'series-state'
    ? { branchId: '1', measure: params.family?.measure ?? 'politicalPower', decimals: 5, states: (params.reads ?? []).map(read => ({ ...read, run: null, drop: null, missing: 'Not recorded' })) }
    : request.product === 'series-summary' ? { branchId: '1', measure: params.family?.measure ?? 'politicalPower', decimals: 5, series: [] }
    : request.product === 'follow' ? { available: true } : {};
  return { sourceId: request.sourceId, revision: request.revision, product: request.product, data };
}

const client = {
  available: true,
  product: async (request: ProductRequest) => { asked.push(request.product); return reply(request); },
  onFollowMessages: async () => () => {},
} as unknown as DashboardClient;

let setHour: (hour: number) => void = () => {};

function Values({ followed }: { followed: boolean }) {
  const [hour, set] = useState(100);
  setHour = set;
  useMeasureSeries(client, source, '1', 'country-measure', 'politicalPower', ['GER', 'SOV'], followed ? null : hour, null, true, false, undefined, 200_000, followed);
  return null;
}

const host = document.body.appendChild(document.createElement('div'));
const root = createRoot(host);

export const fixture = {
  /** `series-state` requests while the hour the widget is shown moves `hours` times, today's way or followed. */
  async readsWhileTheHourMoves(followed: boolean, hours: number) {
    root.render(<Values key={String(followed)} followed={followed} />);
    await settle();
    const before = asked.filter(product => product === 'series-state').length;
    for (let at = 1; at <= hours; at++) {
      setHour(100 + at * 24);
      await settle();
    }
    return asked.filter(product => product === 'series-state').length - before;
  },
  /** The connection's requests: one follow, then a clock that plays for `frames` frames and is paused. */
  async connectionWhileTheClockPlays(frames: number) {
    const clock = new PlaybackController();
    clock.setBounds({ firstHour: 0, lastHour: 100_000 });
    const transport = followTransport(client, source, clock);
    const start = asked.length;
    await transport.follow(1, { product: 'series-state', params: { branchId: '1' } });
    clock.setSpeed(1440);
    clock.setPlaying(true);
    const from = clock.getHour();
    for (let at = 0; at < frames; at++) await frame();
    const hours = clock.getHour() - from;
    clock.setPlaying(false);
    const made = asked.slice(start);
    return { hours, follow: made.filter(product => product === 'follow').length, anchors: made.filter(product => product === 'follow-clock').length,
      other: made.filter(product => product !== 'follow' && product !== 'follow-clock').length };
  },
};
(window as unknown as { requestsFixture: typeof fixture }).requestsFixture = fixture;
