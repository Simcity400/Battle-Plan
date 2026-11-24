import assert from 'node:assert/strict';
import test from 'node:test';
import { readableColour } from '../../charts/palette.ts';
import { engineHour } from '../../dashboard/calendar.ts';
import type { PaintInput } from '../../draw/lane.ts';
import { LaneArtCache } from '../art/laneCache.ts';
import { createLaneArtProvider } from '../art/provider.ts';
import { PENDING_ART, productArt } from '../art/types.ts';

const missingArt = (reason: string) => productArt({ status: 'confirmedMissing', reason });
import { createBoundedReadView, FutureDataError } from '../boundedView.ts';
import type { FamilyTheme } from '../contract.ts';
import { IdentityIndex } from '../identity/index.ts';
import { createSnapshotIdentityProvider } from '../identity/provider.ts';
import { knownValue, notApplicable, notRecorded, partialValue, zeroValue } from '../value.ts';
import {
  assertBarsModelValid,
  countryRowPresentation,
  deriveBarsAudit,
  deriveBarsMirror,
  deriveBarsTooltip,
  FutureBarsDataError,
  InvalidBarsModelError,
  type BarsModel,
} from './model.ts';
import { axisCeiling, layoutFamilyBars } from './layout.ts';
import { drawFamilyBars, hitFamilyBars } from './painter.ts';
import { registerBarsProjection } from './registry.ts';
import { familyBarsLaneView, type FamilyBarsBinding } from './laneView.ts';
import { assertFreshPaint, assertQuietFrames } from '../laneRuntime.fixture.ts';
import { buildShareRowsModel } from './shareRows.ts';

const THEME: FamilyTheme = {
  surface: '#16181d',
  raised: '#1e2128',
  dark: true,
  text: '#eef1f6',
  muted: '#8c95a6',
  line: '#2e3440',
  lineSoft: '#222630',
  accent: '#4f86f7',
  blue: '#4f86f7',
  red: '#e5534b',
  shortage: '#ff6b6f',
  amber: '#d29922',
  green: '#3fb950',
  grey: '#8c95a6',
  hatch: '#6e7681',
  hatchEdge: '#484f58',
  font: 'Inter',
  fontSize: { small: 11, body: 12 },
  lineWidth: 1,
  hatchSpacing: 6,
};

interface RecordedOp {
  readonly op: string;
  readonly args: readonly unknown[];
  readonly fillStyle: unknown;
  readonly strokeStyle: unknown;
}

function makeFakeCtx() {
  const ops: RecordedOp[] = [];
  const stack: { globalAlpha: number; fillStyle: unknown; strokeStyle: unknown; font: string }[] = [];
  const ctx = {
    font: '400 12px Inter',
    fillStyle: '' as unknown,
    strokeStyle: '' as unknown,
    lineWidth: 1,
    globalAlpha: 1,
    textAlign: 'left' as CanvasTextAlign,
    textBaseline: 'middle' as CanvasTextBaseline,
    save() {
      stack.push({ globalAlpha: this.globalAlpha, fillStyle: this.fillStyle, strokeStyle: this.strokeStyle, font: this.font });
    },
    restore() {
      const prev = stack.pop();
      if (prev) Object.assign(this, prev);
    },
    setTransform() {},
    clearRect() {},
    beginPath() {},
    clip() {},
    rect() {},
    moveTo(x: number, y: number) { ops.push({ op: 'moveTo', args: [x, y], fillStyle: this.fillStyle, strokeStyle: this.strokeStyle }); },
    lineTo(x: number, y: number) { ops.push({ op: 'lineTo', args: [x, y], fillStyle: this.fillStyle, strokeStyle: this.strokeStyle }); },
    stroke() { ops.push({ op: 'stroke', args: [], fillStyle: this.fillStyle, strokeStyle: this.strokeStyle }); },
    fill() { ops.push({ op: 'fill', args: [], fillStyle: this.fillStyle, strokeStyle: this.strokeStyle }); },
    fillRect(...args: number[]) { ops.push({ op: 'fillRect', args, fillStyle: this.fillStyle, strokeStyle: this.strokeStyle }); },
    roundRect(...args: unknown[]) { ops.push({ op: 'roundRect', args, fillStyle: this.fillStyle, strokeStyle: this.strokeStyle }); },
    arc(...args: number[]) { ops.push({ op: 'arc', args, fillStyle: this.fillStyle, strokeStyle: this.strokeStyle }); },
    setLineDash(...args: unknown[]) { ops.push({ op: 'setLineDash', args, fillStyle: this.fillStyle, strokeStyle: this.strokeStyle }); },
    drawImage(...args: unknown[]) { ops.push({ op: 'drawImage', args, fillStyle: this.fillStyle, strokeStyle: this.strokeStyle }); },
    fillText(text: string, x: number, y: number) { ops.push({ op: 'fillText', args: [text, x, y], fillStyle: this.fillStyle, strokeStyle: this.strokeStyle }); },
    measureText(text: string) {
      const px = Number.parseFloat(/\b(\d+(?:\.\d+)?)px/.exec(this.font)?.[1] ?? '12') || 12;
      return { width: [...new Intl.Segmenter(undefined, { granularity: 'grapheme' }).segment(text)].length * px * 0.55 };
    },
    createLinearGradient() { return { addColorStop() {} }; },
  };
  return { ctx: ctx as unknown as CanvasRenderingContext2D, ops };
}

test('guards reject future hours, bare nulls, and missing values carrying bar magnitudes', () => {
  const h = engineHour({ year: 1941, month: 9, day: 15, hour: 12 })!;
  assert.throws(
    () => assertBarsModelValid({ form: 'ordered', hour: h + 1, rows: [] }, h),
    FutureBarsDataError,
  );
  assert.throws(
    () => assertBarsModelValid({ form: 'ordered', hour: h, dataHours: [h - 1, h + 2], rows: [] }, h),
    FutureBarsDataError,
  );
  assert.throws(
    () => assertBarsModelValid({
      form: 'ordered',
      hour: h,
      rows: [{
        kind: 'data',
        key: 'GER',
        label: 'German Reich',
        value: { value: notRecorded('Factories table absent') },
        numeric: 293,
      }],
    }, h),
    InvalidBarsModelError,
  );
  assert.throws(
    () => assertBarsModelValid({
      form: 'ordered',
      hour: h,
      rows: [{
        kind: 'data',
        key: 'GER',
        label: 'German Reich',
        value: { value: null as never },
        numeric: 10,
      }],
    }, h),
  );
  const idx = new IdentityIndex({ sourceId: 's', revision: '1', branchId: '1', content: 1 });
  idx.ingestStylesReply({
    branchId: '1',
    hour: h,
    styles: [{
      tag: 'GER',
      name: 'German Reich',
      color: [102, 102, 102],
      valid: [h - 10, h + 10],
      flagChoice: { keys: ['GER_fascism', null, null], reasons: [null, null, null] },
    }],
    warnings: [],
  });
  const artCache = new LaneArtCache();
  const rv = createBoundedReadView({
    hour: h,
    identity: createSnapshotIdentityProvider(idx),
    art: createLaneArtProvider(artCache, { index: idx }),
  });
  assert.throws(() => rv.who('country', 'GER', h + 1), FutureDataError);
  const pres = countryRowPresentation(rv, 'GER');
  assert.equal(pres.label, 'German Reich');
  assert.equal(pres.labelMissing, false);
  assert.equal(pres.color, '#666666');
  assert.equal(pres.media.kind, 'flag');
  assert.equal(pres.media.art.status, 'pending');
});

test('ordered, fuel-against-need, two-sided, share rows, net bars, and stacked share render on one layout and painter', () => {
  const h = engineHour({ year: 1941, month: 9, day: 15, hour: 12 })!;

  const raceModel: BarsModel = {
    form: 'ordered',
    hour: h,
    scaleMax: 293,
    scaleUnit: 'Factories',
    showAxis: true,
    rows: [
      { kind: 'data', key: 'GER', label: 'German Reich', media: { kind: 'flag', art: missingArt('Flag missing'), tag: 'GER' }, color: '#999999', value: { value: knownValue(293), format: 'integer' }, numeric: 293 },
      { kind: 'data', key: 'USA', label: 'United States of America Long Name', media: { kind: 'flag', art: PENDING_ART, tag: 'USA' }, color: '#4f86f7', value: { value: partialValue(292, '1 state unread'), format: 'integer' }, numeric: 292 },
      { kind: 'data', key: 'LUX', label: 'Luxembourg', media: { kind: 'flag', art: missingArt('Flag missing'), tag: 'LUX' }, color: '#3fb950', value: { value: zeroValue(0), format: 'integer' }, numeric: 0 },
      { kind: 'data', key: 'TIB', label: 'Tibet', media: { kind: 'flag', art: missingArt('Flag missing'), tag: 'TIB' }, color: '#d29922', value: { value: notRecorded('Country unread') }, numeric: null },
    ],
  };
  assertBarsModelValid(raceModel, h);
  const raceLayout = layoutFamilyBars(raceModel, 540, 260, THEME);
  const raceDraw = makeFakeCtx();
  const raceStats = drawFamilyBars(raceDraw.ctx, { model: raceModel, layout: raceLayout, theme: THEME, scrollTop: 0, now: 0, glideDuration: 0 });
  assert.equal(raceStats.paintedRows, 4);
  const raceTexts = raceDraw.ops.filter(o => o.op === 'fillText').map(o => String(o.args[0]));
  assert.ok(raceTexts.includes('293'));
  assert.ok(raceTexts.includes('0'));
  assert.ok(raceTexts.includes('Not recorded'));
  assert.equal(raceTexts.includes('1'), false, 'no rank numbers on race rows');
  const hitRow = hitFamilyBars(raceLayout, raceModel, { x: 100, y: raceLayout.bodyTop + 10 }, 0);
  assert.deepEqual(hitRow, { kind: 'row', rowKey: 'GER' });
  const tipMissing = deriveBarsTooltip(raceModel, 'TIB');
  assert.equal(tipMissing?.missingReason, 'Country unread');

  const fuelModel: BarsModel = {
    form: 'ordered',
    hour: h,
    scaleMax: 191,
    showAxis: false,
    rows: [
      {
        kind: 'data',
        key: 'navy',
        label: 'Navy',
        color: '#9085e9',
        value: { value: knownValue(31), format: 'integer' },
        numeric: 31,
        target: { numeric: 42, value: knownValue(42), format: 'integer', shortfall: knownValue(-11), shortfallNumeric: 11 },
        readout: { primary: '31 / 42 a day', shortfall: '\u221211', iconMissing: true },
      },
    ],
  };
  const fuelLayout = layoutFamilyBars(fuelModel, 560, 140, THEME);
  const fuelDraw = makeFakeCtx();
  drawFamilyBars(fuelDraw.ctx, { model: fuelModel, layout: fuelLayout, theme: THEME, scrollTop: 0, now: 0, glideDuration: 0 });
  assert.ok(fuelDraw.ops.some(o => o.op === 'setLineDash' && (o.args[0] as number[]).length > 0), 'unmet need gap uses dashed stroke');
  assert.ok(fuelDraw.ops.some(o => o.op === 'fillText' && o.args[0] === '\u221211' && o.fillStyle === THEME.shortage));

  const butterflyModel: BarsModel = {
    form: 'two-sided',
    hour: h,
    leftHeader: 'Industry received',
    rightHeader: 'Industry sent',
    scaleMax: 6781.5,
    rows: [
      {
        kind: 'data',
        key: 'ETH',
        label: 'Ethiopia',
        color: '#9085e9',
        left: { value: { value: knownValue(6781.5), format: 'decimal1' }, numeric: 6781.5 },
        right: { value: { value: zeroValue(0), format: 'decimal1' }, numeric: 0 },
      },
    ],
  };
  const bfLayout = layoutFamilyBars(butterflyModel, 600, 180, THEME);
  const bfDraw = makeFakeCtx();
  drawFamilyBars(bfDraw.ctx, { model: butterflyModel, layout: bfLayout, theme: THEME, scrollTop: 0, now: 0, glideDuration: 0 });
  const bfTexts = bfDraw.ops.filter(o => o.op === 'fillText').map(o => String(o.args[0]));
  assert.ok(bfTexts.includes('Industry received'));
  assert.ok(bfTexts.includes('Industry sent'));
  assert.ok(bfTexts.includes('0'), 'zero renders as 0, never None yet');
  const bfBar = bfDraw.ops.find(o => o.op === 'roundRect');
  assert.deepEqual(bfBar?.args[4], [4, 0, 0, 4], 'left butterfly bar rounds outer left end and keeps centre square');

  const shareModel = buildShareRowsModel({
    hour: h,
    picked: [
      {
        key: 'reg:poland',
        label: 'Eastern Poland',
        removable: true,
        segments: [
          { key: 'axis', label: 'Axis', share: 0.59, value: knownValue(59), color: '#ccb2a7' },
          { key: 'comintern', label: 'Comintern', share: 0.41, value: knownValue(41), color: '#d44852' },
        ],
        countText: '90 air wings',
      },
    ],
    automatic: [
      {
        key: 'reg:atlantic',
        label: 'North Atlantic Ridge',
        segments: [],
        missing: notRecorded('Air region unread'),
      },
    ],
    addRegionLabel: 'Add region',
  });
  const shareLayoutWide = layoutFamilyBars(shareModel, 680, 220, THEME);
  assert.equal(shareLayoutWide.showReadout, true);
  const shareLayoutMin = layoutFamilyBars(shareModel, 320, 220, THEME);
  assert.equal(shareLayoutMin.showReadout, false, 'count column drops to tooltip at minimum size');
  const shareDraw = makeFakeCtx();
  drawFamilyBars(shareDraw.ctx, { model: shareModel, layout: shareLayoutWide, theme: THEME, scrollTop: 0, now: 0, glideDuration: 0, inspectedKey: 'reg:poland' });
  const shareTexts = shareDraw.ops.filter(o => o.op === 'fillText').map(o => String(o.args[0]));
  assert.ok(shareTexts.includes('Axis 59%'));
  assert.ok(shareTexts.includes('90 air wings'));
  assert.ok(shareTexts.includes('Not recorded'));
  assert.ok(shareTexts.includes('+ Add region'));
  const removeHit = hitFamilyBars(
    shareLayoutWide,
    shareModel,
    { x: shareLayoutWide.nameX + shareLayoutWide.nameWidth - 4, y: shareLayoutWide.bodyTop + shareLayoutWide.rowTops[1]! + shareLayoutWide.rowHeights[1]! / 2 },
    0,
    undefined,
    0,
    0,
    new Set(['reg:poland']),
  );
  assert.deepEqual(removeHit, { kind: 'remove-region', rowKey: 'reg:poland' });
  const hiddenRemove = hitFamilyBars(
    shareLayoutWide,
    shareModel,
    { x: shareLayoutWide.nameX + shareLayoutWide.nameWidth - 4, y: shareLayoutWide.bodyTop + shareLayoutWide.rowTops[1]! + shareLayoutWide.rowHeights[1]! / 2 },
    0,
  );
  assert.equal(hiddenRemove.kind, 'row', 'no remove action where no remove button is drawn');
  const besideButton = hitFamilyBars(
    shareLayoutWide,
    shareModel,
    { x: shareLayoutWide.nameX + shareLayoutWide.nameWidth - 4, y: shareLayoutWide.bodyTop + shareLayoutWide.rowTops[1]! + 1 },
    0,
    undefined,
    0,
    0,
    new Set(['reg:poland']),
  );
  assert.equal(besideButton.kind, 'row', 'the remove action is the button bounds, not the row height');
  const addHit = hitFamilyBars(
    shareLayoutWide,
    shareModel,
    { x: shareLayoutWide.nameX + 10, y: shareLayoutWide.bodyTop + shareLayoutWide.rowTops[4]! + 10 },
    0,
  );
  assert.deepEqual(addHit, { kind: 'add-region', rowKey: '__add-region' });

  const netModel: BarsModel = {
    form: 'net',
    hour: h,
    scaleMax: 13,
    summaryText: '1 short',
    rows: [
      {
        kind: 'data',
        key: 'oil',
        label: 'Oil',
        color: '#c98500',
        value: { value: knownValue(-13), format: { decimals: 0, signed: true }, tone: 'shortage', caption: 'Short a day' },
        numeric: -13,
        pair: { topLabel: 'In', topValue: knownValue(11), bottomLabel: 'Out', bottomValue: knownValue(24), format: 'integer' },
      },
      {
        kind: 'data',
        key: 'coal',
        label: 'Coal',
        color: '#a97b50',
        value: { value: knownValue(4), format: { decimals: 0, signed: true }, caption: 'Surplus a day' },
        numeric: 4,
        pair: { topLabel: 'In', topValue: knownValue(20), bottomLabel: 'Out', bottomValue: knownValue(16), format: 'integer' },
      },
      {
        kind: 'data',
        key: 'tungsten',
        label: 'Tungsten',
        color: '#9085e9',
        value: { value: notApplicable('Mod has no tungsten') },
        numeric: null,
      },
    ],
  };
  const netWide = layoutFamilyBars(netModel, 640, 240, THEME);
  assert.equal(netWide.showPair, true);
  const netMin = layoutFamilyBars(netModel, 320, 220, THEME);
  assert.equal(netMin.showPair, false, 'In/Out pair moves to tooltip at minimum size');
  const netDraw = makeFakeCtx();
  drawFamilyBars(netDraw.ctx, { model: netModel, layout: netWide, theme: THEME, scrollTop: 0, now: 0, glideDuration: 0 });
  const netTexts = netDraw.ops.filter(o => o.op === 'fillText').map(o => String(o.args[0]));
  assert.ok(netTexts.includes('Short a day'));
  assert.ok(netTexts.includes('Surplus a day'));
  assert.ok(netTexts.includes('In'));
  assert.ok(netTexts.includes('Out'));
  assert.ok(netDraw.ops.some(o => o.op === 'roundRect' && o.fillStyle === THEME.shortage));

  const stackedModel: BarsModel = {
    form: 'stacked',
    hour: h,
    rows: [
      {
        kind: 'data',
        key: 'supply',
        label: 'Supply',
        segments: [
          { key: 'full', label: 'Supplied', share: 0.72, value: knownValue(72), color: '#3fb950' },
          { key: 'low', label: 'Low', share: 0.2, value: knownValue(20), color: '#d29922' },
          { key: 'unknown', label: 'Not recorded', share: 0.08, value: notRecorded('No hub read'), color: '#6e7681', hatched: true },
        ],
      },
    ],
    legend: [
      { key: 'full', label: 'Supplied', color: '#3fb950', share: '72%' },
      { key: 'low', label: 'Low', color: '#d29922', share: '20%' },
    ],
  };
  const stackLayout = layoutFamilyBars(stackedModel, 480, 120, THEME);
  const stackDraw = makeFakeCtx();
  drawFamilyBars(stackDraw.ctx, { model: stackedModel, layout: stackLayout, theme: THEME, scrollTop: 0, now: 0, glideDuration: 0 });
  assert.ok(stackDraw.ops.some(o => o.op === 'fillText' && String(o.args[0]).includes('Supplied 72%')));
});

test('lane view scales to 1,500 countries, audits identically, glides at watching speed, and plays top speed without placeholder', () => {
  const h = engineHour({ year: 1941, month: 9, day: 15, hour: 12 })!;
  const rows1500 = Array.from({ length: 1500 }, (_, i) => ({
    kind: 'data' as const,
    key: `C${String(i).padStart(4, '0')}`,
    label: `Country ${i}`,
    color: '#4f86f7',
    value: { value: knownValue(1500 - i), format: 'integer' as const },
    numeric: 1500 - i,
  }));

  registerBarsProjection<{ readonly swapTop?: boolean }>({
    kind: 'test-bars-1500',
    family: 'bars',
    project(readView, opts) {
      const list = opts.swapTop ? [rows1500[1]!, rows1500[0]!, ...rows1500.slice(2)] : rows1500;
      return {
        form: 'ordered',
        hour: readView.hour,
        scaleMax: 1500,
        animateOrder: true,
        rows: list,
      };
    },
  });

  const origOffscreen = globalThis.OffscreenCanvas;
  class FakeOffscreenCanvas {
    width: number;
    height: number;
    private readonly fake = makeFakeCtx();
    constructor(width: number, height: number) { this.width = width; this.height = height; }
    getContext() { return this.fake.ctx; }
  }
  (globalThis as { OffscreenCanvas: unknown }).OffscreenCanvas = FakeOffscreenCanvas;
  try {
    const view = familyBarsLaneView(() => {});
    const state = new Map<string, unknown>();
    let seq = 0;
    const content = () => `c${++seq}`;
    const mkInput = (hour: number, speed: number, now: number, swapTop = false): PaintInput => ({
      widget: 1,
      subs: [],
      pictures: [],
      hour,
      speed,
      now,
      inspect: null,
      binding: { surfaces: [{ width: 480, height: 240, dpr: 1 }] } as unknown as PaintInput['binding'],
      model: {
        family: 'bars',
        kind: 'test-bars-1500',
        options: { swapTop },
        sources: {},
        width: 480,
        height: 240,
        scrollTop: 0,
        theme: THEME,
        audit: true,
        auditId: 7,
      } satisfies FamilyBarsBinding<{ readonly swapTop: boolean }>,
    });

    const first = view.paint(mkInput(h, 24, 1000, false), state, content);
    assert.equal(first.mode, 'picture');
    assert.equal(first.motion, false);
    const auditField = first.fields.audit?.value as { id: number; rows: readonly { row: string; key: string; value?: string }[] };
    assert.equal(auditField.id, 7);
    const expectedAudit = deriveBarsAudit({ form: 'ordered', hour: h, rows: rows1500 });
    assert.deepEqual(auditField.rows, expectedAudit);
    const mirror = deriveBarsMirror({ form: 'ordered', hour: h, rows: rows1500 }, { first: 0, count: 10 });
    assert.equal(mirror.total, 1500);
    assert.equal(mirror.rows.length, 10);

    const watchingSwap = view.paint(mkInput(h + 1, 24, 1050, true), state, content);
    assert.equal(watchingSwap.mode, 'picture');
    assert.equal(watchingSwap.motion, true, 'rank order glides at watching speed');
    assert.ok(watchingSwap.hit?.([100, 18])?.startsWith('row:'));

    const topSpeed = view.paint(mkInput(h + 2, 720, 1100, false), state, content);
    assert.equal(topSpeed.mode, 'picture', 'top speed stays picture mode with no placeholder');
    assert.equal(topSpeed.motion, false, 'top speed disables glide animation');
  } finally {
    (globalThis as { OffscreenCanvas: unknown }).OffscreenCanvas = origOffscreen;
  }
});

test('an ordered scale with an axis ends on the next round tick, so the largest bar stops short of it', () => {
  assert.equal(axisCeiling(293), 300);
  assert.equal(axisCeiling(300), 300);
  assert.equal(axisCeiling(1061), 1500);
  assert.equal(axisCeiling(0), 0);
  const h = engineHour({ year: 1941, month: 9, day: 15, hour: 12 })!;
  const layout = layoutFamilyBars({ form: 'ordered', hour: h, showAxis: true, rows: [{ kind: 'data', key: 'GER', label: 'German Reich', value: { value: knownValue(293) }, numeric: 293 }] }, 600, 300);
  assert.equal(layout.scaleMax, 300);
});

test('audit rows keep one key per row and cell, so two bars never collapse in the verifier', () => {
  const h = engineHour({ year: 1941, month: 9, day: 15, hour: 12 })!;
  const rows = deriveBarsAudit({ form: 'ordered', hour: h, rows: [
    { kind: 'data', key: 'GER', label: 'German Reich', value: { value: knownValue(293) }, numeric: 293 },
    { kind: 'data', key: 'USA', label: 'United States', value: { value: knownValue(292) }, numeric: 292 },
  ] });
  assert.equal(new Map(rows.map(row => [row.key, row.value])).size, rows.length);
  const counted = (primary: string) => new Map(deriveBarsAudit({ form: 'share', hour: h, rows: [
    { kind: 'data', key: '11', label: 'English Channel', segments: [], readout: { primary } },
  ] }).map(row => [row.key, row.value]));
  assert.notDeepEqual(counted('90 air wings'), counted('91 air wings'), 'a painted readout is compared');
});

test('a backward seek drops a cached model from a later hour', () => {
  registerBarsProjection<Record<string, never>>({
    kind: 'test-bars-seek-back',
    family: 'bars',
    project(readView) {
      return { form: 'ordered', hour: readView.hour, rows: [{ kind: 'data', key: 'A', label: readView.hour >= 10 ? 'After' : 'Before', value: { value: knownValue(1) }, numeric: 1 }] };
    },
    nextChange(readView) {
      return readView.hour < 10 ? 10 : Infinity;
    },
  });
  const origOffscreen = globalThis.OffscreenCanvas;
  class FakeOffscreenCanvas {
    width: number;
    height: number;
    private readonly fake = makeFakeCtx();
    constructor(width: number, height: number) { this.width = width; this.height = height; }
    getContext() { return this.fake.ctx; }
  }
  (globalThis as { OffscreenCanvas: unknown }).OffscreenCanvas = FakeOffscreenCanvas;
  try {
    const view = familyBarsLaneView(() => {});
    const state = new Map<string, unknown>();
    let seq = 0;
    const options: Record<string, never> = {};
    const input = (hour: number): PaintInput => ({
      widget: 1, subs: [], pictures: [], hour, speed: 24, now: hour, inspect: null,
      binding: { surfaces: [{ width: 480, height: 240, dpr: 1 }] } as unknown as PaintInput['binding'],
      model: { family: 'bars', kind: 'test-bars-seek-back', options, sources: {}, width: 480, height: 240, scrollTop: 0, theme: THEME, audit: true, auditId: 1 } satisfies FamilyBarsBinding<Record<string, never>>,
    });
    const label = (painted: ReturnType<typeof view.paint>) => (painted.fields.audit?.value as { rows: readonly { key: string; value?: string }[] }).rows.find(row => row.key.endsWith('label'))?.value;
    assert.equal(label(view.paint(input(20), state, () => `c${++seq}`)), 'After');
    assert.equal(label(view.paint(input(5), state, () => `c${++seq}`)), 'Before', 'after seeking back the picture equals a fresh paint');
  } finally {
    (globalThis as { OffscreenCanvas: unknown }).OffscreenCanvas = origOffscreen;
  }
  const options: Record<string, never> = {};
  const binding = { family: 'bars', kind: 'test-bars-seek-back', options, sources: {}, width: 480, height: 240, scrollTop: 0, theme: THEME, reducedMotion: true };
  const surface = { width: 480, height: 240, dpr: 1 };
  assert.equal(assertQuietFrames(() => familyBarsLaneView(() => {}), { label: 'before the change', binding, hour: 3 }, 24, surface), 10);
  assertFreshPaint(() => familyBarsLaneView(() => {}), [
    { label: 'before', binding, hour: 5, check: true },
    { label: 'after', binding, hour: 20, speed: 24 },
    { label: 'seek back', binding, hour: 5, check: true },
    { label: 'scrolled', binding: { ...binding, scrollTop: 10 }, hour: 5, check: true },
  ], surface);
});

test('the top painted gliding row takes the hit where two rows overlap', () => {
  const h = engineHour({ year: 1941, month: 9, day: 15, hour: 12 })!;
  const model: BarsModel = { form: 'ordered', hour: h, rows: [
    { kind: 'data', key: 'A', label: 'A', value: { value: knownValue(2) }, numeric: 2 },
    { kind: 'data', key: 'B', label: 'B', value: { value: knownValue(1) }, numeric: 1 },
  ] };
  const layout = layoutFamilyBars(model, 480, 240);
  const glide = new Map([['B', { fromY: layout.bodyTop + layout.rowTops[0]!, startedAt: 0 }]]);
  const y = layout.bodyTop + layout.rowTops[0]! + 4;
  const hit = hitFamilyBars(layout, model, { x: 200, y }, 0, glide, 0, 1000);
  assert.equal(hit.kind === 'row' ? hit.rowKey : null, 'B');
});

test('a known 40 percent share beside an unavailable part fills 40 percent and marks the rest unavailable', () => {
  const h = engineHour({ year: 1941, month: 9, day: 15, hour: 12 })!;
  const model: BarsModel = { form: 'share', hour: h, rows: [{ kind: 'data', key: 'R', label: 'Region', segments: [
    { key: 'AX', label: 'Axis', share: 0.4, value: knownValue(4), color: '#aa3344' },
    { key: 'UN', label: 'Unrecorded side', share: 0, value: notRecorded('Score unknown'), color: '#777777', hatched: true },
  ] }] };
  const layout = layoutFamilyBars(model, 640, 200);
  const { ctx, ops } = makeFakeCtx();
  drawFamilyBars(ctx, { model, layout, theme: THEME, scrollTop: 0, now: 0, glideDuration: 0 });
  const known = ops.filter(op => op.op === 'fillRect' && op.fillStyle === readableColour('#aa3344', THEME.surface));
  assert.equal(known.length, 1);
  assert.ok(Math.abs((known[0]!.args[2] as number) - layout.plotWidth * 0.4) < 2, 'the known part is drawn at its own fraction');
});

test('a hover over 1,500 rows reuses navigation, meta and row bounds, and the mirror holds only visible rows', () => {
  const rows = Array.from({ length: 1500 }, (_, i) => ({ kind: 'data' as const, key: `K${i}`, label: `Country ${i}`, value: { value: knownValue(1500 - i) }, numeric: 1500 - i }));
  registerBarsProjection<Record<string, never>>({
    kind: 'test-bars-hover',
    family: 'bars',
    project(readView) { return { form: 'ordered', hour: readView.hour, rows }; },
  });
  const origOffscreen = globalThis.OffscreenCanvas;
  class FakeOffscreenCanvas {
    width: number;
    height: number;
    private readonly fake = makeFakeCtx();
    constructor(width: number, height: number) { this.width = width; this.height = height; }
    getContext() { return this.fake.ctx; }
  }
  (globalThis as { OffscreenCanvas: unknown }).OffscreenCanvas = FakeOffscreenCanvas;
  try {
    const view = familyBarsLaneView(() => {});
    const state = new Map<string, unknown>();
    const options: Record<string, never> = {};
    let seq = 0;
    const input = (inspect: string | null, now: number): PaintInput => ({
      widget: 1, subs: [], pictures: [], hour: 50, speed: 24, now, inspect,
      binding: { surfaces: [{ width: 480, height: 240, dpr: 1 }] } as unknown as PaintInput['binding'],
      model: { family: 'bars', kind: 'test-bars-hover', options, sources: {}, width: 480, height: 240, scrollTop: 0, theme: THEME } satisfies FamilyBarsBinding<Record<string, never>>,
    });
    const first = view.paint(input(null, 0), state, () => `c${++seq}`);
    const hovered = view.paint(input('row:K3', 16), state, () => `c${++seq}`);
    assert.equal(hovered.fields.nav, first.fields.nav);
    assert.equal(hovered.fields.meta, first.fields.meta);
    assert.equal(hovered.fields.rowBounds, first.fields.rowBounds);
    const mirror = first.fields.mirror?.value as { rows: readonly unknown[]; total: number };
    assert.equal(mirror.total, 1500);
    assert.ok(mirror.rows.length < 40, `mirror holds ${mirror.rows.length} rows`);
  } finally {
    (globalThis as { OffscreenCanvas: unknown }).OffscreenCanvas = origOffscreen;
  }
});

test('bar fills are the identity colour raised to 3:1 against the surface, at full opacity', () => {
  const h = engineHour({ year: 1941, month: 9, day: 15, hour: 12 })!;
  const dark = '#2a2a2a';
  const model: BarsModel = { form: 'ordered', hour: h, rows: [{ kind: 'data', key: 'GER', label: 'German Reich', color: dark, value: { value: knownValue(5) }, numeric: 5 }] };
  const layout = layoutFamilyBars(model, 480, 200);
  const { ctx, ops } = makeFakeCtx();
  drawFamilyBars(ctx, { model, layout, theme: THEME, scrollTop: 0, now: 0, glideDuration: 0 });
  const fills = ops.filter(op => op.op === 'fill').map(op => op.fillStyle);
  assert.ok(fills.includes(readableColour(dark, THEME.surface)));
  assert.ok(!fills.includes(dark), 'a near-invisible map colour is never drawn raw');
});

test('a two-sided row can carry a ratio column with its own header', () => {
  const h = engineHour({ year: 1941, month: 9, day: 15, hour: 12 })!;
  const model: BarsModel = { form: 'two-sided', hour: h, leftHeader: 'Own losses', rightHeader: 'Enemy aircraft destroyed', readoutHeader: 'Ratio', rows: [
    { kind: 'data', key: 'GER', label: 'German Reich', color: '#888888', left: { value: { value: knownValue(2474) }, numeric: 2474 }, right: { value: { value: knownValue(4453) }, numeric: 4453 }, readout: { primary: '1.80 : 1' } },
  ] };
  const layout = layoutFamilyBars(model, 840, 300);
  assert.ok(layout.showReadout && layout.plotRight < layout.readoutX);
  const { ctx, ops } = makeFakeCtx();
  drawFamilyBars(ctx, { model, layout, theme: THEME, scrollTop: 0, now: 0, glideDuration: 0 });
  const texts = ops.filter(op => op.op === 'fillText').map(op => String(op.args[0]));
  assert.ok(texts.includes('1.80 : 1') && texts.includes('Ratio'));
});
