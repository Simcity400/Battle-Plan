import { Profiler } from 'react';
import type { ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import type { DashboardClient, DashboardSource, ProductRequest } from './client';
import type { BoundTrack, WidgetSpec } from './model';
import type { WidgetRuntime } from './definitions';
import { PlaybackController } from './playback';
import { PlaybackStrip } from './controls';
import CountryChip from './CountryChip';
import { CountriesSection } from './settings/CountriesSection';
import MapWidget from '../widgets/map/MapWidget';
import { useIdentityHour } from './atHead.ts';
import { askTimeline } from './countryTimeline.ts';
import { usePlaybackHour } from './follow/useFollow.ts';
import { firstHourOfMonth, gameDate, monthNames } from './calendar.ts';
import { useCountriesAt } from './useCountriesAt.ts';

const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function settle() { for (let i = 0; i < 6; i++) await frame(); }

const RENAMED = 5000;
const LAST = 100_000;

const asked: string[] = [];
const source = { id: 'lease', path: 'campaign.battleplan', revision: 'r1', metadata: { campaignKey: 'c', playerCountryId: null,
  branches: [{ id: '1', firstHour: 0, lastHour: LAST }], countries: [{ id: '0', tag: 'GER', name: 'Germany', observedBranchId: '1', observedHour: 0 }] } } as unknown as DashboardSource;
const track = { id: 't', roleId: 'role', label: 'Campaign', selectedTag: 'GER', axisOffset: 0, branch: { id: '1', firstHour: 0, lastHour: LAST, gaps: [] }, source } as unknown as BoundTrack;
const unheld = { ...track, branch: { ...track.branch, id: '2' } } as BoundTrack;

const span = { firstHour: 0, lastHour: LAST };
const GROUPS = { branchId: '1', warnings: [], humans: [], wars: [], warStarts: [], capitulations: [], identityChanges: [], countryIdentities: [],
  unobserved: { humans: [], factions: [], wars: [] },
  factions: [{ id: '1', ...span, name: 'Axis Powers', nameReason: null, recordedName: 'Axis', members: [{ ...span, tag: 'GER' }], leaders: [{ ...span, tag: 'GER' }], incomplete: [],
    names: [{ firstHour: 0, lastHour: RENAMED - 1, name: 'Axis', reason: null }, { firstHour: RENAMED, lastHour: LAST, name: 'Axis Powers', reason: null }] }] };
const FACTS = { branchId: '1', known: [[0, LAST]], names: ['Germany', 'German Reich'], reasons: [], flags: [], warnings: [],
  countries: [{ tag: 'GER', runs: [[0, 0, 0x336699, -1, -1], [RENAMED, 1, 0x336699, -1, -1]] }] };

const IDENTITIES = { branchId: '1', countries: [{ tag: 'GER', identities: [{ firstHour: 0, lastHour: LAST, slot: 0, countryId: 0 }], unresolved: [] }] };

const client = {
  available: true, nativeMaps: true,
  product: async (request: ProductRequest) => {
    asked.push(request.product);
    const data = request.product === 'country-groups' ? GROUPS : request.product === 'country-facts' ? FACTS
      : request.product === 'country-identities' ? IDENTITIES
        : { available: true };
    return { sourceId: request.sourceId, revision: request.revision, product: request.product, data };
  },
} as unknown as DashboardClient;

const clock = new PlaybackController();
clock.setBounds({ firstHour: 0, lastHour: LAST });
const maps = { register: () => () => {}, paint: () => () => {}, retry() {}, schedule() {}, subscribe: () => () => {}, getState: () => null,
  subscribeHidden: () => () => {}, isHidden: () => false };
const runtime = { client, clock, maps, tracks: [track], campaignTracks: [track], defaultRoleId: 'role' } as unknown as WidgetRuntime;
const faction: WidgetSpec = { id: 'chip', kind: 'political-power', configVersion: 1, title: 'Power', campaign: 'default', trackIds: ['t'], time: { mode: 'follow' },
  countries: { kind: 'group', group: { type: 'faction', id: '1' }, membership: 'hour', aggregate: false }, options: {} } as unknown as WidgetSpec;
const map = { ...faction, id: 'map', kind: 'map', countries: { kind: 'all' } } as unknown as WidgetSpec;

const renders: Record<string, number> = {};
const counted = (id: string, child: ReactNode) => <Profiler id={id} onRender={() => { renders[id] = (renders[id] ?? 0) + 1; }}>{child}</Profiler>;
const sameHour = (hour: number) => hour;

function Identity({ of }: { of: BoundTrack }) {
  const hour = useIdentityHour(client, of, ['GER'], clock, true, sameHour);
  return <span data-identity={of.branch.id}>{hour}</span>;
}

function Fallback({ on }: { on: boolean }) {
  return <span data-fallback>{usePlaybackHour(clock, on)}</span>;
}

const host = document.body.appendChild(document.createElement('div'));
const root = createRoot(host);

function Lanes({ held }: { held: number }) {
  const { country } = useCountriesAt(client, track, ['GER'], held, true, ['GER'], undefined, undefined, { clock, on: true, hour: sameHour });
  return <span data-lanes>{country('GER').name}</span>;
}

const slotTrack = { ...track, selectedTag: null, selectedSlot: 0 } as unknown as BoundTrack;
const slotRuntime = { ...runtime, tracks: [slotTrack], campaignTracks: [slotTrack] } as unknown as WidgetRuntime;
const byTag = { ...faction, id: 'by-tag', countries: { kind: 'tags', tags: ['GER'] } } as unknown as WidgetSpec;
const bySlot = { ...faction, id: 'by-slot', countries: { kind: 'selected' } } as unknown as WidgetSpec;
const countryHost = document.body.appendChild(document.createElement('div'));
const countryRoot = createRoot(countryHost);

const JANUARY_1936 = 1936 * 12;
const calendarClock = new PlaybackController();
const calendarFirst = firstHourOfMonth(JANUARY_1936), calendarLast = firstHourOfMonth(JANUARY_1936 + 120);
calendarClock.setBounds({ firstHour: calendarFirst, lastHour: calendarLast });
const calendarTrack = { ...track, id: 'c', roleId: 'calendar', branch: { ...track.branch, id: '3', firstHour: calendarFirst, lastHour: calendarLast } } as BoundTrack;
const calendarHost = document.body.appendChild(document.createElement('div'));
const calendarRoot = createRoot(calendarHost);
const popover = () => document.querySelector('.date-time-popover:popover-open');
const button = (selector: string) => { const found = document.querySelector<HTMLButtonElement>(selector); if (!found) throw new Error(`no ${selector}`); return found; };

export const fixture = {
  async mount(hour: number) {
    askTimeline(client, { sourceId: 'lease', revision: 'r1', branchId: '1', content: 0 }, source, hour);
    clock.seek(hour);
    root.render(<>
      {counted('strip', <PlaybackStrip clock={clock} tracks={[track]} alignment="calendar" defaultRoleId="role" />)}
      {counted('chip', <CountryChip widget={faction} runtime={runtime} active />)}
      {counted('section', <CountriesSection widget={faction} reference={track} runtime={runtime} limits={{ max: 10, ranked: false, aggregate: false } as never} onChange={() => {}} />)}
      {counted('map', <MapWidget widget={map} runtime={runtime} active mapOptions={{ fog: true } as never} headerSlot={null} onDiagnostics={() => {}} />)}
      {counted('identity', <Identity of={track} />)}
      {counted('unheld', <Identity of={unheld} />)}
      {counted('fallback', <Fallback on />)}
      {counted('off', <Fallback on={false} />)}
    </>);
    await settle();
  },
  async play(speed: number, frames: number) {
    clock.setSpeed(speed);
    clock.setPlaying(true);
    await frame(); await frame();
    const before = { ...renders }, asking = asked.length, from = clock.getHour(), started = performance.now();
    const dates: string[] = [];
    let changes = 0, last = clock.getHour();
    const stop = clock.subscribeFrame(() => {
      if (clock.getHour() === last) return;
      last = clock.getHour();
      changes++;
      dates.push(document.querySelector('.header-date .date-time-value')?.textContent ?? '');
    });
    for (let at = 0; at < frames; at++) await frame();
    stop();
    const during = Object.fromEntries(Object.keys(renders).map(id => [id, (renders[id] ?? 0) - (before[id] ?? 0)]));
    const requests = asked.slice(asking);
    const elapsed = performance.now() - started, to = clock.getHour();
    clock.setPlaying(false);
    await settle();
    return { changes, from, to, elapsed, renders: during, requests, dates: new Set(dates).size,
      chip: document.querySelector('.dash-widget-country')?.textContent ?? '', identity: document.querySelector('[data-identity="1"]')?.textContent ?? '' };
  },
  async seek(hour: number) {
    const before = { ...renders };
    clock.seek(hour);
    await settle();
    return Object.fromEntries(Object.keys(renders).map(id => [id, (renders[id] ?? 0) - (before[id] ?? 0)]));
  },
  async mountCountries(held: number) {
    countryRoot.render(<>
      <span data-chip="tag">{counted('tagChip', <CountryChip widget={byTag} runtime={runtime} active />)}</span>
      <span data-chip="slot">{counted('slotChip', <CountryChip widget={bySlot} runtime={slotRuntime} active />)}</span>
      {counted('lanes', <Lanes held={held} />)}
    </>);
    await settle();
  },
  countries: () => ({ tag: document.querySelector('[data-chip="tag"] .dash-widget-country')?.textContent ?? '',
    slot: document.querySelector('[data-chip="slot"] .dash-widget-country')?.textContent ?? '',
    lanes: document.querySelector('[data-lanes]')?.textContent ?? '' }),
  async mountCalendar(month: number) {
    calendarClock.seek(firstHourOfMonth(JANUARY_1936 + month));
    calendarRoot.render(counted('calendar', <PlaybackStrip clock={calendarClock} tracks={[calendarTrack]} alignment="calendar" defaultRoleId="calendar" />));
    await settle();
  },
  async toggleCalendar() {
    const trigger = calendarHost.querySelector<HTMLButtonElement>('.header-date .date-time-trigger');
    if (!trigger) throw new Error('no date picker');
    trigger.click();
    await settle();
  },
  calendar() {
    const body = popover();
    const selected = body?.querySelectorAll('[role="gridcell"][aria-selected="true"]') ?? [];
    const date = gameDate(calendarClock.getHour())!;
    return { open: !!body?.querySelector('.calendar-grid'), month: body?.querySelector('.calendar-grid')?.getAttribute('aria-label') ?? '',
      days: [...selected].map(cell => Number(cell.querySelector('[data-day]')?.getAttribute('data-day'))),
      time: body?.querySelector('.calendar-time .select-trigger > span')?.textContent ?? '',
      head: { month: `${monthNames[date.month - 1]} ${date.year}`, day: date.day, hour: date.hour } };
  },
  async playCalendar(speed: number, frames: number) {
    calendarClock.setSpeed(speed);
    calendarClock.setPlaying(true);
    await frame(); await frame();
    const before = renders.calendar ?? 0, from = calendarClock.getHour();
    const seen: { head: number; shown: number[] }[] = [];
    let last = calendarClock.getHour();
    const stop = calendarClock.subscribeFrame(() => {
      if (calendarClock.getHour() === last) return;
      last = calendarClock.getHour();
      const shown = [...popover()?.querySelectorAll('[role="gridcell"][aria-selected="true"] [data-day]') ?? []].map(day => Number(day.getAttribute('data-day')));
      seen.push({ head: gameDate(last)!.day, shown });
    });
    for (let at = 0; at < frames; at++) await frame();
    stop();
    const during = (renders.calendar ?? 0) - before, to = calendarClock.getHour();
    calendarClock.setPlaying(false);
    await settle();
    return { renders: during, changes: seen.length, from: gameDate(from), to: gameDate(to), seen };
  },
  async startCalendar(speed: number, frames: number) {
    calendarClock.setSpeed(speed);
    calendarClock.setPlaying(true);
    for (let at = 0; at < frames; at++) await frame();
  },
  async pauseCalendar() { calendarClock.setPlaying(false); await settle(); },
  async nextMonth() { button('.date-time-popover:popover-open [aria-label="Next month"]').click(); await settle(); },
  async pickDay(day: number) { button(`.date-time-popover:popover-open [data-day="${day}"]`).click(); await settle(); },
};
(window as unknown as { atHeadFixture: typeof fixture }).atHeadFixture = fixture;
