import assert from 'node:assert/strict';
import test from 'node:test';
import { EMPTY_VIEW } from '../../charts/view.ts';
import type { ChartTokens } from '../../charts/tokens.ts';
import type { PaintInput } from '../../draw/lane.ts';
import type { SeriesHeadReply } from '../../generated/dashboard';
import { timeChartLaneView, type TimeChartBinding } from './laneView.ts';
import { seriesKey } from './tiles.ts';

function fakeTokens(): ChartTokens {
  return {
    surface: '#111111',
    raised: '#1a1a1a',
    text: '#eeeeee',
    muted: '#888888',
    border: '#333333',
    grid: '#222222',
    axis: '#aaaaaa',
    accent: '#4488ff',
    positive: '#22aa44',
    warning: '#ddaa22',
    error: '#dd3333',
    blueprint: '#224488',
    blueprintBorder: '#335599',
    blueprintText: '#ffffff',
    font: 'sans-serif',
    fontSize: { small: 11, body: 12, title: 14 },
    lineWidth: { grid: 1, series: 2, emphasis: 3 },
    hatchSpacing: 6,
    gapOpacity: 0.3,
    motion: 'snappy',
    reducedMotion: false,
    countryColours: 'fixed',
    categories: ['#4488ff', '#ff8844'],
  };
}

class StubCanvas {
  width: number;
  height: number;
  draws = 0;
  private readonly ctx: Record<string, unknown>;

  constructor(width: number, height: number) {
    this.width = width;
    this.height = height;
    const noop = () => {};
    this.ctx = {
      font: '',
      fillStyle: '',
      strokeStyle: '',
      lineWidth: 1,
      lineJoin: 'round',
      globalAlpha: 1,
      textAlign: 'left',
      textBaseline: 'alphabetic',
      setTransform: noop,
      clearRect: () => { this.draws++; },
      save: noop,
      restore: noop,
      beginPath: noop,
      closePath: noop,
      moveTo: noop,
      lineTo: noop,
      rect: noop,
      clip: noop,
      stroke: noop,
      fill: noop,
      fillRect: noop,
      strokeRect: noop,
      fillText: noop,
      arc: noop,
      setLineDash: noop,
      measureText: (text: string) => ({ width: text.length * 6 }),
    };
  }

  getContext() {
    return this.ctx;
  }
}

function withStubOffscreen<T>(run: (created: StubCanvas[]) => T): T {
  const created: StubCanvas[] = [];
  const prev = (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas;
  (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas = class extends StubCanvas {
    constructor(w: number, h: number) {
      super(w, h);
      created.push(this);
    }
  };
  try {
    return run(created);
  } finally {
    (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas = prev;
  }
}

function headReply(hour: number, binFirstHour: number, val: number): SeriesHeadReply {
  return {
    branchId: '1',
    measure: 'politicalPower',
    decimals: 5,
    hour,
    binHours: 6,
    binFirstHour,
    binLastHour: binFirstHour + 5,
    closedThrough: binFirstHour - 1,
    lines: [{
      country: 'GER',
      openBin: {
        firstHour: binFirstHour,
        lastHour: hour,
        firstValue: val,
        lastValue: val,
        lastValueHour: hour,
        minHour: binFirstHour,
        minValue: val,
        maxHour: hour,
        maxValue: val,
        firstBreak: false,
        hasBreak: false,
      },
      value: { valueRaw: String(val * 100_000), value: val },
      missing: [],
      marks: { before: '0', count: 1 },
    }],
  };
}

function makeBinding(overrides: Partial<TimeChartBinding> = {}): TimeChartBinding {
  return {
    family: 'time-chart',
    kind: 'political-power',
    tokens: fakeTokens(),
    extent: [0, 720],
    branchStart: 0,
    lastHour: 720,
    playback: 'draw-in',
    afterPlayhead: false,
    windowHours: 365 * 24,
    paused: false,
    playheadMode: 'follow',
    staticPlayhead: null,
    view: EMPTY_VIEW,
    axes: { xFrom: null, xTo: null, yMin: null, yMax: null, log: false },
    formatValue: 'powerAxis',
    areaFill: false,
    endLabels: true,
    endLabelFormat: { decimals: 5, digits: 0, unit: 'PP' },
    spendTicks: true,
    series: [{ key: seriesKey('GER'), country: 'GER', role: 'primary', label: 'Germany', color: '#4488ff' }],
    headSubId: 1,
    tilesVersion: 1,
    ...overrides,
  };
}

function makeInput(model: TimeChartBinding, head: SeriesHeadReply, hour: number): PaintInput {
  return {
    widget: 1,
    hour,
    speed: 1440,
    now: 0,
    inspect: null,
    binding: { generation: 1, surfaces: [{ width: 400, height: 220, dpr: 1 }], offset: 0, pin: null },
    subs: [{ id: 1, channel: 'panel', product: 'series-head', route: 0 }],
    pictures: [{ parts: [{ value: { kind: 'panel', bytes: null, data: head, refused: null }, from: hour, to: hour }] }] as unknown as PaintInput['pictures'],
    model,
  };
}

test('timeChartLaneView skips redraw on unchanged inputs and emits audit only when active', () => {
  withStubOffscreen(created => {
    const view = timeChartLaneView();
    const state = new Map<string, unknown>();
    let seq = 1;
    const nextContent = () => `c${seq++}`;
    const head1 = headReply(12, 12, 100);
    const binding1 = makeBinding({ audit: false });

    const first = view.paint(makeInput(binding1, head1, 12), state, nextContent);
    assert.equal(created.length, 1);
    assert.equal(created[0]!.draws, 1);
    assert.equal(first.fields.audit, undefined);
    assert.ok(first.fields.legend);
    assert.ok(first.fields.viewport);

    const second = view.paint(makeInput(makeBinding({ audit: false }), head1, 12), state, nextContent);
    assert.equal(created[0]!.draws, 1);
    assert.equal(second.surfaces[0]!.content, first.surfaces[0]!.content);

    const third = view.paint(makeInput(makeBinding({ audit: true, auditId: 7 }), headReply(15, 12, 125), 15), state, nextContent);
    assert.equal(created[0]!.draws, 2);
    assert.notEqual(third.surfaces[0]!.content, first.surfaces[0]!.content);
    const audit = third.fields.audit?.value as { id: number; rows: readonly { key: string; value?: string }[] };
    assert.equal(audit.id, 7);
    assert.ok(audit.rows.some(r => r.key === 'scale'));
    assert.ok(audit.rows.some(r => r.key === `series:${seriesKey('GER')}` && r.value?.includes('12500000')));

    view.unmount?.(1, state);
    assert.equal(state.has('time-chart'), false);
  });
});
