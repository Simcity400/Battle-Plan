import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, type ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { dayLabel } from '../../charts/draw.ts';
import { mountModule } from '../../dashboard/pendingGroups.fixture.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { BoundTrack } from '../../dashboard/model';
import type { WeatherReply } from './weatherData';

const choices = { dayHours: 24, regions: [{ id: '1', name: 'Alps' }], flags: [{ id: 'rain', name: 'Rain' }, { id: 'snow', name: 'Snow' }],
  modifiers: [{ id: 'air', name: 'Air detection' }], temperature: 'available', warnings: [] };
const window = { firstHour: 0, lastHour: 239, known: [], reads: [], series: [] };
/** Ten days in two five-day columns; the second column's first day holds snow. */
const folded: WeatherReply = { ...choices, window, columns: { days: 5,
  columns: [{ firstHour: 0, lastHour: 119, sampledHour: 119, readHour: 119, recorded: 5 }, { firstHour: 120, lastHour: 239, sampledHour: 239, readHour: 239, recorded: 5 }],
  regions: [{ id: '1', observed: [5, 5], clear: [5, 3], measures: [{ id: 'rain', days: [0, 1] }, { id: 'snow', days: [0, 1] }] }] } };
/** The same ten days one a column. */
const daily: WeatherReply = { ...choices, window, columns: { days: 1,
  columns: Array.from({ length: 10 }, (_, day) => ({ firstHour: day * 24, lastHour: day * 24 + 23, sampledHour: day * 24 + 23, readHour: day * 24 + 20, recorded: 1 })),
  regions: [{ id: '1', observed: Array(10).fill(1), clear: Array.from({ length: 10 }, (_, day) => day === 5 ? 0 : 1),
    measures: [{ id: 'snow', days: Array.from({ length: 10 }, (_, day) => day === 5 ? 1 : 0) }] }] } };
const snowDay: WeatherReply = { ...choices, modifiers: [], window: { firstHour: 120, lastHour: 143, known: [{ firstHour: 120, lastHour: 143 }], reads: [[120, 143, 140]],
  series: [{ name: '1|snow', runs: [[120, 143, '1', true]] }] } };

interface Fixture { data: WeatherReply; chart?: TimeChartProps; read: Map<number, { data?: WeatherReply; error?: string }>; wanted: number[];
  menu?: unknown[]; drilled?: number }
const host = globalThis as typeof globalThis & { __weather?: Fixture };

async function mount() {
  return mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/weather-by-region/WeatherWidget.tsx', {
    '/widgets/weather-by-region/useWeather.ts': `export const useWeatherInView = () => ({ options: {}, show: 'conditions', modifier: null, data: globalThis.__weather.data,
        retry() {}, onSummaryView() {} });
      export const useWeatherDays = () => ({ get: day => globalThis.__weather.read.get(day), want: day => globalThis.__weather.wanted.push(day), hover() {} });`,
    '/charts/TimeChart.tsx': `export default props => { globalThis.__weather.chart = props; return null; };`,
    '/charts/jump.ts': 'export const jumpItem = (_runtime, _track, hour) => ({ hour });',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open: (_x, _y, items) => { globalThis.__weather.menu = items; }, menu: null });',
    '/widgets/weather-by-region/RegionProvinces.tsx': 'export default props => { globalThis.__weather.drilled = props.hour; return null; };',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 239; export const usePlaybackSpeed = () => 1;',
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({ categories: ['red', 'blue'], surface: 'surface', text: 'text', accent: 'accent', error: 'error',
      gapOpacity: 0.2, muted: 'muted', border: 'border', font: 'sans', fontSize: { small: 11 }, lineWidth: { grid: 1, series: 2 }, hatch: { spacing: 4 } });
      export const useViewportHeight = () => 800;`,
  });
}
const propsFor = (lastHour: number) => ({ widget: { id: 'weather', kind: 'weather-by-region', trackIds: ['main'], time: { mode: 'campaign' }, options: {} },
  runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1' }, branch: { id: '1', firstHour: 0, lastHour } }],
    clock: { getHour: () => lastHour } }, active: true, onDiagnostics() {} }) as unknown as WidgetProps;
const props = { widget: { id: 'weather', kind: 'weather-by-region', trackIds: ['main'], time: { mode: 'campaign' }, options: {} },
  runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1' }, branch: { id: '1', firstHour: 0, lastHour: 239 } }],
    clock: { getHour: () => 239 } }, active: true, onDiagnostics() {} } as unknown as WidgetProps;

test('a folded column keeps the hovered day: its tooltip, menu and drill read that day, not the column\'s last', async () => {
  const fixture: Fixture = { data: folded, read: new Map(), wanted: [] };
  host.__weather = fixture;
  const { module, close } = await mount();
  try {
    const render = () => { renderToStaticMarkup(createElement(module.default, props)); return fixture.chart!; };
    const chart = render();
    // The pointer at the column's first day stays there.
    const hour = chart.snap!(125, undefined as never);
    assert.equal(hour, 125);
    const loading = chart.rows!.tooltip(0, hour)!;
    assert.deepEqual(loading.rows.slice(0, 2).map(row => [row.label, row.value]), [[`${dayLabel(120)} to ${dayLabel(239)}`, undefined], ['Recorded days', '5 of 5']],
      'the column summary shows at once');
    assert.deepEqual(loading.rows.at(-1), { label: dayLabel(125), value: 'Loading…', divider: true });
    assert.deepEqual(fixture.wanted, [5], 'the hovered day is asked for, not the column\'s last');
    // Another day's read never stands in for the hovered one.
    fixture.read.set(6, { data: snowDay });
    render().rows!.tooltip(0, 125);
    assert.deepEqual(fixture.wanted, [5, 5]);
    fixture.read.set(5, { data: { ...snowDay, coverage: { firstHour: 0, lastHour: 123, complete: false }, pending: [{ firstHour: 124, lastHour: 143 }] } as WeatherReply });
    assert.deepEqual(render().rows!.tooltip(0, 125)!.rows.at(-1), { label: dayLabel(125), value: 'Still being prepared', divider: true },
      'a day preparation has not reached is never shown as not recorded');
    assert.deepEqual(fixture.wanted, [5, 5, 5], 'a day read while still being prepared is asked for again');
    fixture.read.set(5, { data: snowDay });
    const exact = render().rows!.tooltip(0, 125)!;
    assert.deepEqual(fixture.wanted, [5, 5, 5], 'a finished day is not asked for again');
    const text = exact.rows.map(row => `${row.label}=${row.value ?? ''}`).join(';');
    assert.match(text, /Rain=1 day;Snow=1 day;No condition=3 days/);
    assert.match(text, /Conditions=Snow/);
    assert.match(text, new RegExp(`As of=${dayLabel(140)}`));
    fixture.read.delete(5);
    render().onContextMenu!(125, 0, 0);
    assert.deepEqual(fixture.menu, [{ hour: 125 }], 'an unread folded day jumps to its own hour, not the column\'s read');
    render().onPick!(125);
    const { pickedDay } = await import('./weatherData.ts');
    assert.equal(pickedDay(125, 24, [0, 239]), 143, 'the drill opens the hovered day, not the last day of its column');
    assert.equal(pickedDay(239, 24, [0, 230]), 230);
  } finally { delete host.__weather; await close(); }
});

test('hours the shown reply does not reach yet are pending, never clear', async () => {
  const fixture: Fixture = { data: folded, read: new Map(), wanted: [] };
  host.__weather = fixture;
  const { module, close } = await mount();
  try {
    // The view now reaches past the reply still shown while the next one loads.
    const markup = renderToStaticMarkup(createElement(module.default, propsFor(479)));
    assert.match(markup, /Still being prepared/, 'the unreached hours draw as pending');
    const tip = fixture.chart!.rows!.tooltip(0, 300)!;
    assert.deepEqual(tip.rows.map(row => row.label), [dayLabel(300), 'Loading…']);
    assert.deepEqual(fixture.wanted, []);
  } finally { delete host.__weather; await close(); }
});

test('one-day columns snap to their read and show their tooltip without asking for anything', async () => {
  const fixture: Fixture = { data: daily, read: new Map(), wanted: [] };
  host.__weather = fixture;
  const { module, close } = await mount();
  try {
    renderToStaticMarkup(createElement(module.default, props));
    const chart = fixture.chart!;
    assert.equal(chart.snap!(125, undefined as never), 140);
    const tooltip = chart.rows!.tooltip(0, 140)!;
    assert.deepEqual(tooltip.rows.map(row => [row.label, row.value]), [[dayLabel(140), undefined], ['Conditions', 'Snow'],
      ['Temperature (average of provinces)', 'Not available'], ['As of', dayLabel(140)]]);
    assert.deepEqual(fixture.wanted, []);
  } finally { delete host.__weather; await close(); }
});

test('the playhead is this campaign\'s hour, not the comparison clock\'s', async () => {
  const fixture: Fixture = { data: daily, read: new Map(), wanted: [] };
  host.__weather = fixture;
  const { module, close } = await mount();
  try {
    // Aligned 100 hours later than the clock: clock hour 239 is this campaign's hour 339.
    const shifted = { ...props, widget: { ...props.widget, time: { mode: 'follow' } },
      runtime: { ...props.runtime, tracks: [{ ...props.runtime.tracks[0], axisOffset: 100, branch: { id: '1', firstHour: 0, lastHour: 479 } }] } } as unknown as WidgetProps;
    renderToStaticMarkup(createElement(module.default, shifted));
    assert.equal(fixture.chart!.playhead, 339);
  } finally { delete host.__weather; await close(); }
});

test('a resize asks nothing again where the days fit, and one day reads as the history does', async () => {
  const built = await import('../../dashboard/widgetRequests.ts');
  const track = props.runtime.tracks[0] as BoundTrack;
  const widget = props.widget;
  assert.deepEqual(built.weatherRequest(widget, track, [0, 239], 512), built.weatherRequest(widget, track, [0, 239], 1024));
  assert.equal((built.weatherRequest(widget, track, [0, 239], 512)!.params as { width: number }).width, 10);
  assert.deepEqual(built.weatherDayRequest(widget, track, 5, 24).params, { branchId: '1', firstHour: 120, lastHour: 143, daily: true, show: 'conditions' });
  assert.deepEqual(built.weatherDayRequest(widget, track, 9, 24).params, { branchId: '1', firstHour: 216, lastHour: 239, daily: true, show: 'conditions' });
});
