import { StrictMode, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { flushSync } from 'react-dom';
import TooltipLayer from '../../charts/TooltipLayer.tsx';
import { tooltipStore, tooltipText } from '../../charts/tooltip.ts';
import type { DashboardClient, ProductRequest } from '../../dashboard/client.ts';
import type { FamilyViewProps, WidgetRuntime } from '../../dashboard/definitions.ts';
import { createWidget, type BoundTrack } from '../../dashboard/model.ts';
import { PlaybackController } from '../../dashboard/playback.ts';
import type { WarContributionHistoryReply, WarObservation } from '../../generated/dashboard';
import { MEASURE_LIST } from './definition.ts';
import { warContributionMeasureDeclaration } from './measureProjection.ts';
import { warContributionSidesDeclaration } from './sidesProjection.ts';
import Widget from './index.tsx';
import '../../styles.css';

const firstHour = 60810000, lastHour = firstHour + 48, pause = firstHour + 24;
const branch = { id: '1', firstHour, lastHour: lastHour + 24, gaps: [{ firstHour: pause, lastHour: pause, reason: 'Recording paused in the fixture' }] };
const tags = ['AAA', 'BBB', 'CCC', 'DDD', 'EEE', 'FFF', 'GGG'];
const observation = { viewpointTag: 'AAA', viewpointSide: tags.slice(0, 6), opposingSide: ['GGG'], attackersAreViewpoint: true,
  representative: ['AAA', 'GGG'], name: { text: 'Fixture War', known: true, key: null, reason: null } } satisfies Omit<WarObservation, 'firstHour' | 'lastHour'>;
const war = { id: '0', firstHour, lastHour, startedBy: true, endedBy: false,
  observations: [{ ...observation, firstHour, lastHour: pause - 1 }, { ...observation, firstHour: pause + 1, lastHour }], participants: [], capitulations: [] };
const measureKeys = MEASURE_LIST.map(({ key }) => key) as WarContributionHistoryReply['measureKeys'];
const contributors = tags.map((tag, slot) => ({ slot, id: slot + 1,
  membership: [[firstHour, pause - 1, slot === 6 ? 1 : 0, tag], [pause + 1, lastHour, slot === 6 ? 1 : 0, tag]],
  amounts: measureKeys.map(() => [[firstHour, '200000']]), partialCasualties: [{ firstHour, lastHour }],
})) as WarContributionHistoryReply['contributors'];
const side = (sideTags: readonly string[]) => ({ total: String(200000 * sideTags.length), shareOfWar: 5000, partial: false,
  members: sideTags.map(tag => ({ tag, value: '200000', shareOfSide: Math.round(10000 / sideTags.length), shareOfWar: 1429, partial: false })) });
const snapshot = { viewpointSide: observation.viewpointSide, opposingSide: observation.opposingSide, reason: null, participants: [],
  measures: measureKeys.map(key => ({ key, viewpoint: side(observation.viewpointSide), opposing: side(observation.opposingSide) })) };
const source = { id: 'fixture', revision: '1', path: '', metadata: { campaignKey: 'fixture', branches: [branch], playerCountryId: '0' } };
const track = { id: 'main', roleId: 'primary', label: 'Main', branchIndex: 0, anchorOffset: 0, source, branch, axisOffset: 0, selectedSlot: 0 } as unknown as BoundTrack;
const calls: { call: ProductRequest; signal: AbortSignal; answer: (data: WarContributionHistoryReply) => void }[] = [];
const snapshotHours: number[] = [];
const client = { available: true, async product(call: ProductRequest, signal: AbortSignal) {
  const reply = (data: unknown) => ({ sourceId: call.sourceId, revision: call.revision, product: call.product, data });
  if (call.product === 'war-contribution-history') return new Promise(resolve => calls.push({ call, signal, answer: data => resolve(reply(data)) }));
  if (call.product === 'wars') return reply({ branchId: branch.id, available: true, warnings: [], countryNames: [], unobserved: branch.gaps, capitulationsUnobserved: [], viewpoints: [], wars: [war] });
  if (call.product === 'war-contributions') {
    const hour = (call.params as { hour: number }).hour;
    snapshotHours.push(hour);
    return reply({ branchId: branch.id, available: true, hour, warnings: [], missing: null, contributions: [snapshot] });
  }
  if (call.product === 'country-identities') return reply({ branchId: branch.id, known: [], countries: tags.map((tag, slot) => ({ tag, identities: [{ slot, countryId: slot + 1, firstHour, lastHour: branch.lastHour }], unresolved: [] })), unknown: [], warnings: [] });
  if (call.product === 'country-identity') return reply({ branchId: branch.id, countries: [], warnings: [] });
  return reply({ branchId: branch.id, known: [], countries: [], unknown: [], warnings: [] });
} } as unknown as DashboardClient;
const clock = new PlaybackController();
clock.setBounds(branch); clock.seek(branch.lastHour);
const runtime = { client, tracks: [track], campaignTracks: [track], clock, alignment: 'calendar' } as unknown as WidgetRuntime;
const widget = { ...createWidget('war-contribution', track.id, 'history-fixture'), time: { mode: 'pin' as const, windows: { [track.id]: { firstOffset: 0, lastOffset: branch.lastHour - firstHour } } } };
const container = document.body.appendChild(document.createElement('main'));
container.style.cssText = 'width: 1000px; height: 500px;';
const root = createRoot(container);
let pick = '';

function Fixture({ view }: { view: 'measure' | 'sides' }) {
  const [measure, setMeasure] = useState('casualties');
  return <><Widget.render {...{ widget, view, runtime, active: true, options: { measure },
    setOption: (_key: string, next: string) => { pick = next; setMeasure(next); },
    manifestDeclaration: view === 'measure' ? warContributionMeasureDeclaration : warContributionSidesDeclaration } as unknown as FamilyViewProps} /><TooltipLayer /></>;
}

const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function wait(check: () => boolean) {
  while (!check()) await frame();
}

function detail() { return document.querySelector<HTMLElement>('.war-history:popover-open'); }
const totals = (amount: string) => measureKeys.map(() => [[firstHour, amount]]) as WarContributionHistoryReply['sides'][number]['totals'];

Object.assign(window, { warHistoryFixture: {
  async mount(view: 'measure' | 'sides') {
    flushSync(() => root.render(<StrictMode><Fixture key={view} view={view} /></StrictMode>));
    await wait(() => !!container.querySelector('[data-row-key="viewpoint:AAA"]'));
  },
  async activateTable(key: string) {
    const row = container.querySelector<HTMLTableRowElement>(`tr[data-row-key="${key}"]`);
    if (!row) throw new Error(`Table row ${key} missing`);
    row.focus();
  },
  async chart() {
    await wait(() => !!detail()?.querySelector('canvas'));
    return { focus: document.activeElement?.className, text: detail()?.querySelector('caption')?.textContent,
      rows: detail()?.querySelectorAll('tbody tr').length };
  },
  async loading(count: number, title?: string) {
    await wait(() => calls.length === count && !!detail() && (!title || detail()?.querySelector('strong')?.textContent?.includes(title) === true));
    return { calls: calls.map(({ call, signal }) => [call.params, signal.aborted]), title: detail()?.querySelector('strong')?.textContent, text: detail()?.querySelector('[role="status"]')?.textContent };
  },
  answer(index: number, unavailable = false) {
    const reason = 'WarParticipation: the fixture’s ledger was unavailable';
    const missing = unavailable ? [{ firstHour, lastHour, reason }] : [
      { firstHour: firstHour + 1, lastHour: pause - 1, reason }, { firstHour: pause + 1, lastHour: pause + 1, reason },
    ];
    calls[index]!.answer({ war, measureKeys, contributors, gaps: branch.gaps, warnings: [], sides: [
      { totals: totals('1200000'), missing }, { totals: totals('200000'), missing },
    ] });
  },
  read: () => ({ text: detail()?.innerText, caption: detail()?.querySelector('caption')?.textContent,
    spoken: detail()?.querySelector('caption .sr-only')?.textContent, count: calls.length, snapshotHours, pick }),
  async closed() {
    await wait(() => !detail() && !!container.querySelector(':focus'));
    return calls.map(({ signal }) => signal.aborted);
  },
  async change(index: number, value: string) {
    const select = detail()!.querySelectorAll('select')[index]!;
    flushSync(() => { select.value = value; select.dispatchEvent(new Event('change', { bubbles: true })); });
  },
  async nextFrames() { await frame(); await frame(); },
  compare() { flushSync(() => detail()!.querySelector<HTMLButtonElement>('button[aria-pressed]')!.click()); },
  tooltip: () => { const tip = tooltipStore.get(); return tip ? tooltipText(tip.content) : null; },
  hitsBody(x: number, y: number) { return document.elementFromPoint(x, y)?.closest('.fh-body') != null; },
  box(selector: string) { const { x, y, width, height } = document.querySelector(selector)!.getBoundingClientRect(); return { x, y, width, height }; },
  lastHour, pause,
} });
