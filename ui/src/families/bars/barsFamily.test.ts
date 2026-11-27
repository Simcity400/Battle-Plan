import assert from 'node:assert/strict';
import { resolveKitTokens } from '../../draw/kit.ts';
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
import { knownValue, notApplicable, notRecorded, partialValue, pendingValue, zeroValue } from '../value.ts';
import { buildTooltipContent } from '../host/tooltips.ts';
import { tooltipText } from '../../charts/tooltip.ts';
import {
  assertBarsModelValid,
  formatBarsValueField,
  countryRowPresentation,
  deriveBarsAudit,
  deriveBarsMirror,
  deriveBarsTooltip,
  FutureBarsDataError,
  InvalidBarsModelError,
  type BarsModel, type BarsProjection,
} from './model.ts';

import { axisCeiling, barsScrollHeight, layoutFamilyBars, visibleBarsRange } from './layout.ts';
import { barsHostAdapter } from './hostAdapter.ts';
import { drawFamilyBars, hitFamilyBars } from './painter.ts';
import { registerFamilyProjection } from '../registry.ts';
import { barsLane, type FamilyBarsBinding } from './laneView.ts';
import { assertContinuity, assertFreshPaint, assertQuietFrames, assertTooltipDated, firstRow, LaneTransport } from '../laneRuntime.fixture.ts';
import { createLaneView } from '../laneRuntime.ts';
import { buildShareRowsModel } from './shareRows.ts';

test('R3-S5: both sides of a bars tooltip retain exact partial values and reasons', () => {
  const left = { value: partialValue(17.2468, 'Left coverage missing'), format: 'decimal1' as const };
  const right = { value: partialValue(31.8642, 'Right coverage missing'), format: 'decimal1' as const };
  const model: BarsModel = { form: 'two-sided', hour: 7, rows: [{ kind: 'data', key: 'A01', label: knownValue('A01'),
    left: { value: left, numeric: 17.2468 }, right: { value: right, numeric: 31.8642 } }] };
  assert.equal(formatBarsValueField(left).text, '17.2');
  assert.equal(formatBarsValueField(right).text, '31.8');
  const items = deriveBarsTooltip(model, 'A01')!.items!;
  assert.deepEqual(items.map(item => [item.value, item.partialReason]), [
    ['17.2468', 'Left coverage missing'], ['31.8642', 'Right coverage missing'],
  ]);
  assert.match(tooltipText(buildTooltipContent(deriveBarsTooltip(model, 'A01')!)), /Left coverage missing/);
  assert.match(tooltipText(buildTooltipContent(deriveBarsTooltip(model, 'A01')!)), /Right coverage missing/);
});
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
        label: knownValue('German Reich'),
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
        label: knownValue('German Reich'),
        value: { value: null as never },
        numeric: 10,
      }],
    }, h),
  );
  assert.throws(
    () => assertBarsModelValid({
      form: 'ordered',
      hour: h,
      rows: [{
        kind: 'data',
        key: 'GER',
        value: { value: knownValue(10) },
        numeric: 10,
      }],
    }, h),
    InvalidBarsModelError,
  );
  assertBarsModelValid({
    form: 'two-sided',
    twoSidedLayout: 'split-sides',
    hour: h,
    rows: [{
      kind: 'data',
      key: 'left:none:0',
      rightSide: {
        key: 'right:GER',
        label: knownValue('German Reich'),
        value: { value: knownValue(10) },
        numeric: 10,
      },
    }],
  }, h);
  assert.throws(
    () => assertBarsModelValid({
      form: 'two-sided',
      twoSidedLayout: 'split-sides',
      hour: h,
      rows: [{
        kind: 'data',
        key: 'left:none:0',
        numeric: 100,
        rightSide: {
          key: 'right:GER',
          label: knownValue('German Reich'),
          value: { value: knownValue(10) },
          numeric: 10,
        },
      }],
    }, h),
    InvalidBarsModelError,
  );
  assert.throws(
    () => assertBarsModelValid({
      form: 'two-sided',
      twoSidedLayout: 'split-sides',
      hour: h,
      rows: [{
        kind: 'data',
        key: 'left:none:0',
        tooltip: { title: 'Ghost', hour: h },
        rightSide: {
          key: 'right:GER',
          label: knownValue('German Reich'),
          value: { value: knownValue(10) },
          numeric: 10,
        },
      }],
    }, h),
    InvalidBarsModelError,
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
  assert.deepEqual(pres.label, knownValue('German Reich'));
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
      { kind: 'data', key: 'GER', label: knownValue('German Reich'), media: { kind: 'flag', art: missingArt('Flag missing'), tag: 'GER' }, color: '#999999', value: { value: knownValue(293), format: 'integer' }, numeric: 293 },
      { kind: 'data', key: 'USA', label: knownValue('United States of America Long Name'), media: { kind: 'flag', art: PENDING_ART, tag: 'USA' }, color: '#4f86f7', value: { value: partialValue(292, '1 state unread'), format: 'integer' }, numeric: 292 },
      { kind: 'data', key: 'LUX', label: knownValue('Luxembourg'), media: { kind: 'flag', art: missingArt('Flag missing'), tag: 'LUX' }, color: '#3fb950', value: { value: zeroValue(0), format: 'integer' }, numeric: 0 },
      { kind: 'data', key: 'TIB', label: knownValue('Tibet'), media: { kind: 'flag', art: missingArt('Flag missing'), tag: 'TIB' }, color: '#d29922', value: { value: notRecorded('Country unread') }, numeric: null },
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
        label: knownValue('Navy'),
        color: '#9085e9',
        value: { value: knownValue(31), format: 'integer' },
        numeric: 31,
        target: { numeric: 42, value: knownValue(42), format: 'integer', shortfall: knownValue(-11), shortfallNumeric: 11 },
        readout: { primary: '31 / 42 a day', shortfall: '\u221211' },
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
        label: knownValue('Ethiopia'),
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
        label: knownValue('Eastern Poland'),
        region: 40,
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
        label: knownValue('North Atlantic Ridge'),
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
  assert.ok(shareTexts.includes('Add region'));
  const gripHit = hitFamilyBars(
    shareLayoutWide,
    shareModel,
    { x: shareLayoutWide.nameX - 4, y: shareLayoutWide.bodyTop + shareLayoutWide.rowTops[1]! + shareLayoutWide.rowHeights[1]! / 2 },
    0,
    undefined,
    0,
    0,
    new Set(['reg:poland']),
  );
  assert.deepEqual(gripHit, { kind: 'grip-region', rowKey: 'reg:poland' });
  assert.equal(hitFamilyBars(shareLayoutWide, shareModel, { x: shareLayoutWide.nameX - 4, y: shareLayoutWide.bodyTop + shareLayoutWide.rowTops[3]! + 4 }, 0, undefined, 0, 0, new Set(['reg:atlantic'])).kind, 'row',
    'a row that is not a region has no grip');
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
        label: knownValue('Oil'),
        color: '#c98500',
        value: { value: knownValue(-13), format: { decimals: 0, signed: true }, tone: 'shortage', caption: 'Short a day' },
        numeric: -13,
        pair: { topLabel: 'In', topValue: knownValue(11), bottomLabel: 'Out', bottomValue: knownValue(24), format: 'integer' },
      },
      {
        kind: 'data',
        key: 'coal',
        label: knownValue('Coal'),
        color: '#a97b50',
        value: { value: knownValue(4), format: { decimals: 0, signed: true }, caption: 'Surplus a day' },
        numeric: 4,
        pair: { topLabel: 'In', topValue: knownValue(20), bottomLabel: 'Out', bottomValue: knownValue(16), format: 'integer' },
      },
      {
        kind: 'data',
        key: 'tungsten',
        label: knownValue('Tungsten'),
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
  assert.ok(netDraw.ops.some(o => o.op === 'roundRect' && o.fillStyle === resolveKitTokens(THEME).shortFill), 'a bar below zero takes the shortage fill');

  const stackedModel: BarsModel = {
    form: 'stacked',
    hour: h,
    rows: [
      {
        kind: 'data',
        key: 'supply',
        label: knownValue('Supply'),
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
    label: knownValue(`Country ${i}`),
    color: '#4f86f7',
    value: { value: knownValue(1500 - i), format: 'integer' as const },
    numeric: 1500 - i,
  }));

  registerFamilyProjection({
    kind: 'test-bars-1500',
    family: 'bars',
    project(readView) {
      const list = readView.hour === h + 1 ? [rows1500[1]!, rows1500[0]!, ...rows1500.slice(2)] : rows1500;
      return {
        form: 'ordered',
        hour: readView.hour,
        scaleMax: 1500,
        animateOrder: true,
        rows: list,
      };
    },
    nextChange: readView => readView.hour + 1,
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
    const view = createLaneView(barsLane, () => {});
    const state = new Map<string, unknown>();
    let seq = 0;
    const content = () => `c${++seq}`;
    const model = new LaneTransport().carry({
      family: 'bars',
      kind: 'test-bars-1500',
      options: {},
      sources: {},
      width: 480,
      height: 240,
      scrollTop: 0,
      theme: THEME,
      audit: true,
      auditId: 7,
    } satisfies FamilyBarsBinding<Record<string, never>>);
    const mkInput = (hour: number, speed: number, now: number): PaintInput => ({
      widget: 1,
      subs: [],
      pictures: [],
      hour,
      seek: 0,
      speed,
      now,
      inspect: null,
      binding: { generation: 1, surfaces: [{ width: 480, height: 240, dpr: 1 }], offset: 0, pin: null },
      model,
    });

    const first = view.paint(mkInput(h, 24, 1000), state, content);
    assert.equal(first.mode, 'picture');
    assert.equal(first.motion, false);
    const auditField = first.fields.audit?.value as { id: number; rows: readonly { row: string; key: string; value?: string }[] };
    assert.equal(auditField.id, 7);
    const painted = Object.fromEntries(Object.entries(first.fields).map(([slot, field]) => [slot, (field as { value: unknown }).value]));
    const exact = (scrollTop: number, height = 240) => barsHostAdapter.deriveAuditRows!({ form: 'ordered', hour: h, rows: rows1500 }, h, {
      viewport: { width: 480, height, dpr: 1, scrollLeft: 0, scrollTop, zoomWindow: null }, settings: undefined, expanded: {}, theme: THEME, fields: painted,
    });
    assert.deepEqual(auditField.rows, exact(0), 'the installed window and the exact read at the same scroll agree');
    assert.deepEqual(exact(0, 400), auditField.rows, 'the exact audit reads the painted geometry; it never lays the chart out again');
    assert.deepEqual(barsHostAdapter.deriveAuditRows!({ form: 'ordered', hour: h, rows: rows1500 }, h, {
      viewport: { width: 480, height: 240, dpr: 1, scrollLeft: 0, scrollTop: 0, zoomWindow: null }, settings: undefined, expanded: {}, theme: THEME, fields: {},
    }), [], 'with no installed picture nothing is certified as painted');
    assert.ok(new Set(auditField.rows.map(row => row.row)).size < 20, 'a ten-row viewport audits its visible rows, not 1,500 countries');
    assert.notDeepEqual(exact(600), auditField.rows, 'a picture scrolled to the wrong window fails its audit');
    const mirror = deriveBarsMirror({ form: 'ordered', hour: h, rows: rows1500 }, { first: 0, count: 10 });
    assert.equal(mirror.total, 1500);
    assert.equal(mirror.rows.length, 10);

    const watchingSwap = view.paint(mkInput(h + 1, 24, 1050), state, content);
    assert.equal(watchingSwap.mode, 'picture');
    assert.equal(watchingSwap.motion, true, 'rank order glides at watching speed');
    assert.ok(watchingSwap.hit?.([100, 18])?.startsWith('row:'));

    const topSpeed = view.paint(mkInput(h + 2, 720, 1100), state, content);
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
  const layout = layoutFamilyBars({ form: 'ordered', hour: h, showAxis: true, rows: [{ kind: 'data', key: 'GER', label: knownValue('German Reich'), value: { value: knownValue(293) }, numeric: 293 }] }, 600, 300);
  assert.equal(layout.scaleMax, 300);
});

test('audit rows keep one key per row and cell, so two bars never collapse in the verifier', () => {
  const h = engineHour({ year: 1941, month: 9, day: 15, hour: 12 })!;
  const rows = deriveBarsAudit({ form: 'ordered', hour: h, rows: [
    { kind: 'data', key: 'GER', label: knownValue('German Reich'), value: { value: knownValue(293) }, numeric: 293 },
    { kind: 'data', key: 'USA', label: knownValue('United States'), value: { value: knownValue(292) }, numeric: 292 },
  ] });
  assert.equal(new Map(rows.map(row => [row.key, row.value])).size, rows.length);
  const counted = (primary: string) => new Map(deriveBarsAudit({ form: 'share', hour: h, rows: [
    { kind: 'data', key: '11', label: knownValue('English Channel'), segments: [], readout: { primary } },
  ] }).map(row => [row.key, row.value]));
  assert.notDeepEqual(counted('90 air wings'), counted('91 air wings'), 'a painted readout is compared');
});

test('a backward seek drops a cached model from a later hour', () => {
  registerFamilyProjection<BarsProjection<Record<string, never>>>({
    kind: 'test-bars-seek-back',
    family: 'bars',
    project(readView) {
      return { form: 'ordered', hour: readView.hour, rows: [{ kind: 'data', key: 'A', label: knownValue(readView.hour >= 10 ? 'After' : 'Before'), value: { value: knownValue(1) }, numeric: 1 }] };
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
    const view = createLaneView(barsLane, () => {});
    const state = new Map<string, unknown>();
    let seq = 0;
    const options: Record<string, never> = {};
    const model = new LaneTransport().carry({ family: 'bars', kind: 'test-bars-seek-back', options, sources: {}, width: 480, height: 240, scrollTop: 0, theme: THEME, audit: true, auditId: 1 } satisfies FamilyBarsBinding<Record<string, never>>);
    const input = (hour: number): PaintInput => ({
      widget: 1, subs: [], pictures: [], hour, seek: 0, speed: 24, now: hour, inspect: null,
      binding: { surfaces: [{ width: 480, height: 240, dpr: 1 }] } as unknown as PaintInput['binding'],
      model,
    });
    const label = (painted: ReturnType<typeof view.paint>) => (painted.fields.audit?.value as { rows: readonly { key: string; value?: string }[] }).rows.find(row => row.key.endsWith('label'))?.value;
    assert.equal(label(view.paint(input(20), state, () => `c${++seq}`)), '["known",["str","After"]]');
    assert.equal(label(view.paint(input(5), state, () => `c${++seq}`)), '["known",["str","Before"]]', 'after seeking back the picture equals a fresh paint');
  } finally {
    (globalThis as { OffscreenCanvas: unknown }).OffscreenCanvas = origOffscreen;
  }
  const options: Record<string, never> = {};
  const binding = { family: 'bars', kind: 'test-bars-seek-back', options, sources: {}, width: 480, height: 240, scrollTop: 0, theme: THEME, reducedMotion: true };
  const surface = { width: 480, height: 240, dpr: 1 };
  assert.equal(assertQuietFrames(() => createLaneView(barsLane, () => {}), { label: 'before the change', binding, hour: 3 }, 24, surface), 10);
  assertTooltipDated(() => createLaneView(barsLane, () => {}), { label: 'bars', binding, hour: 3 }, firstRow, 5, surface);
  assertFreshPaint(() => createLaneView(barsLane, () => {}), [
    { label: 'before', binding, hour: 5, check: true },
    { label: 'after', binding, hour: 20, speed: 24 },
    { label: 'seek back', binding, hour: 5, check: true },
    { label: 'scrolled', binding: { ...binding, scrollTop: 10 }, hour: 5, check: true },
  ], surface);
  registerFamilyProjection<BarsProjection<Record<string, never>>>({
    kind: 'test-bars-seek-swap',
    family: 'bars',
    project(readView) {
      const a = { kind: 'data' as const, key: 'A', label: knownValue('A'), value: { value: knownValue(readView.hour >= 10 ? 1 : 2) }, numeric: readView.hour >= 10 ? 1 : 2 };
      const b = { kind: 'data' as const, key: 'B', label: knownValue('B'), value: { value: knownValue(readView.hour >= 10 ? 2 : 1) }, numeric: readView.hour >= 10 ? 2 : 1 };
      return { form: 'ordered', hour: readView.hour, animateOrder: true, rows: readView.hour >= 10 ? [b, a] : [a, b] };
    },
    nextChange: readView => (readView.hour < 10 ? 10 : Infinity),
  });
  const swapping = { ...binding, kind: 'test-bars-seek-swap', reducedMotion: false };
  assertFreshPaint(() => createLaneView(barsLane, () => {}), [
    { label: 'swapped later', binding: swapping, hour: 3, speed: 24 },
    { label: 'swapped later', binding: swapping, hour: 20, speed: 24 },
    { label: 'seek back across the swap while playing', binding: swapping, hour: 5, speed: 24, check: true },
    { label: 'seek forward across the swap while playing', binding: swapping, hour: 30, speed: 24, check: true },
  ], surface);
  assertContinuity(() => createLaneView(barsLane, () => {}), {
    from: { label: 'before the swap', binding: swapping, hour: 9 },
    to: { label: 'the swap', binding: swapping, hour: 10 },
  }, surface);
});

test('the top painted gliding row takes the hit where two rows overlap', () => {
  const h = engineHour({ year: 1941, month: 9, day: 15, hour: 12 })!;
  const model: BarsModel = { form: 'ordered', hour: h, rows: [
    { kind: 'data', key: 'A', label: knownValue('A'), value: { value: knownValue(2) }, numeric: 2 },
    { kind: 'data', key: 'B', label: knownValue('B'), value: { value: knownValue(1) }, numeric: 1 },
  ] };
  const layout = layoutFamilyBars(model, 480, 240);
  const glide = new Map([['B', { fromY: layout.bodyTop + layout.rowTops[0]!, startedAt: 0 }]]);
  const y = layout.bodyTop + layout.rowTops[0]! + 4;
  const hit = hitFamilyBars(layout, model, { x: 200, y }, 0, glide, 0, 1000);
  assert.equal(hit.kind === 'row' ? hit.rowKey : null, 'B');
});

test('a known 40 percent share beside an unavailable part fills 40 percent and marks the rest unavailable', () => {
  const h = engineHour({ year: 1941, month: 9, day: 15, hour: 12 })!;
  const model: BarsModel = { form: 'share', hour: h, rows: [{ kind: 'data', key: 'R', label: knownValue('Region'), segments: [
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
  const rows = Array.from({ length: 1500 }, (_, i) => ({ kind: 'data' as const, key: `K${i}`, label: knownValue(`Country ${i}`), value: { value: knownValue(1500 - i) }, numeric: 1500 - i }));
  registerFamilyProjection<BarsProjection<Record<string, never>>>({
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
    const view = createLaneView(barsLane, () => {});
    const state = new Map<string, unknown>();
    const options: Record<string, never> = {};
    let seq = 0;
    const model = new LaneTransport().carry({ family: 'bars', kind: 'test-bars-hover', options, sources: {}, width: 480, height: 240, scrollTop: 0, theme: THEME } satisfies FamilyBarsBinding<Record<string, never>>);
    const input = (inspect: string | null, now: number): PaintInput => ({
      widget: 1, subs: [], pictures: [], hour: 50, seek: 0, speed: 24, now, inspect, model,
      binding: { surfaces: [{ width: 480, height: 240, dpr: 1 }] } as unknown as PaintInput['binding'],
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
  const model: BarsModel = { form: 'ordered', hour: h, rows: [{ kind: 'data', key: 'GER', label: knownValue('German Reich'), color: dark, value: { value: knownValue(5) }, numeric: 5 }] };
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
    { kind: 'data', key: 'GER', label: knownValue('German Reich'), color: '#888888', left: { value: { value: knownValue(2474) }, numeric: 2474 }, right: { value: { value: knownValue(4453) }, numeric: 4453 }, readout: { primary: '1.80 : 1' } },
  ] };
  const layout = layoutFamilyBars(model, 840, 300);
  assert.ok(layout.showReadout && layout.plotRight < layout.readoutX);
  const { ctx, ops } = makeFakeCtx();
  drawFamilyBars(ctx, { model, layout, theme: THEME, scrollTop: 0, now: 0, glideDuration: 0 });
  const texts = ops.filter(op => op.op === 'fillText').map(op => String(op.args[0]));
  assert.ok(texts.includes('1.80 : 1') && texts.includes('Ratio'));
});

test('a measure list sits left of the butterfly: the body moves right, a list row hits as a pick with its key, and the audit names the chosen one', async () => {
  const { hitPickList, pickAudit, pickNav, pickRowTops } = await import('./pickList.ts');
  const { parseBarsHit } = await import('./hostAdapter.ts');
  const pickList = {
    rows: [
      { key: 'one', label: 'Invented measure one', left: { fraction: 0.25 }, right: { fraction: 0.75 } },
      { key: 'two', label: 'Invented measure two', left: { fraction: null }, right: { fraction: null }, divided: true },
    ],
    selectedKey: 'two',
  };
  const model = {
    form: 'two-sided' as const, twoSidedLayout: 'split-sides' as const, hour: 10, pickList,
    rows: [{ kind: 'data' as const, key: 'a', label: knownValue('Country A'), value: { value: knownValue(0.5), format: 'share' as const }, numeric: 0.5 }],
  };
  const wide = layoutFamilyBars(model, 1600, 320);
  assert.equal(wide.pickWidth, 330);
  assert.ok(wide.bodyLeft > wide.pickWidth && wide.nameX > wide.bodyLeft && wide.centerX > wide.bodyLeft, 'the butterfly is laid out right of the list');
  const tops = pickRowTops(pickList);
  assert.ok(tops[1]! - tops[0]! > 22, 'a divided row keeps its gap');
  assert.equal(hitPickList(pickList, wide, { x: 40, y: tops[1]! + 4 }), 'pick:two');
  assert.equal(hitPickList(pickList, wide, { x: wide.bodyLeft + 40, y: tops[0]! + 4 }), null, 'the butterfly is not the list');
  assert.deepEqual(parseBarsHit('pick:two')?.meta, { pick: 'two' }, 'activation carries the measure');
  assert.deepEqual(pickNav(pickList).map(item => item[0]), ['pick:one', 'pick:two']);
  assert.ok(pickAudit(pickList).some(row => row.row === 'pick:two' && JSON.parse(row.value as string)[3] === true));
  assert.equal(layoutFamilyBars(model, 500, 320).pickWidth, 0, 'too narrow for the list: the butterfly alone');

  registerFamilyProjection({ kind: 'test-bars-pick', family: 'bars', project: readView => ({ ...model, hour: readView.hour }), nextChange: () => Infinity });
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
    for (const width of [500, 1600]) {
      const view = createLaneView(barsLane, () => {});
      let seq = 0;
      const binding = new LaneTransport().carry({ family: 'bars', kind: 'test-bars-pick', options: {}, sources: {}, width, height: 320, scrollTop: 0, theme: THEME, audit: true, auditId: 3 } satisfies FamilyBarsBinding<Record<string, never>>);
      const paint = view.paint({ widget: 1, subs: [], pictures: [], hour: 10, seek: 0, speed: 1, now: 0, inspect: null,
        binding: { generation: 1, surfaces: [{ width, height: 320, dpr: 1 }], offset: 0, pin: null }, model: binding }, new Map(), () => `c${++seq}`);
      const lane = (paint.fields.audit?.value as { rows: readonly { row: string }[] }).rows;
      const fields = Object.fromEntries(Object.entries(paint.fields).map(([slot, field]) => [slot, (field as { value: unknown }).value]));
      const exact = barsHostAdapter.deriveAuditRows!(model, 10, {
        viewport: { width, height: 320, dpr: 1, scrollLeft: 0, scrollTop: 0, zoomWindow: null }, settings: undefined, expanded: {}, theme: THEME, fields,
      });
      const picks = (rows: readonly { row: string }[]) => rows.filter(row => row.row.startsWith('pick:')).length;
      const shown = width === 1600 ? 2 : 0;
      assert.equal(picks(lane), shown, `the lane audits the picker only where it is painted (${width} px)`);
      assert.equal(picks(exact), shown, `the exact audit follows the installed picker (${width} px)`);
    }
  } finally {
    (globalThis as { OffscreenCanvas: unknown }).OffscreenCanvas = origOffscreen;
  }
});

test('at the deepest scroll the extent allows, the last row sits above the axis, not under it', () => {
  const rows = Array.from({ length: 40 }, (_, at) => ({ kind: 'data' as const, key: `C${at}`, label: knownValue(`C${at}`), value: { value: knownValue(100 - at) }, numeric: 100 - at }));
  const layout = layoutFamilyBars({ form: 'ordered', hour: 0, rows, showAxis: true }, 480, 240);
  assert.ok(layout.axisHeight > 0);
  const deepest = barsScrollHeight(layout) - layout.height;
  const lastBottom = layout.rowTops[39]! + layout.rowHeights[39]!;
  assert.ok(lastBottom - deepest <= layout.bodyHeight, 'the last row ends inside the clipped body');
  assert.equal(visibleBarsRange(layout, deepest, 0).last, 40);
});

test('share rows draw the category legend strip, wrap long category lists cleanly, and draw right-aligned count unit once', () => {
  const h = engineHour({ year: 1941, month: 9, day: 15, hour: 12 })!;
  const model: BarsModel = {
    form: 'share',
    hour: h,
    scaleUnit: 'Battalions',
    legend: [
      { key: 'armored', label: 'Armored Battalions', color: '#3b82f6' },
      { key: 'combat-support', label: 'Combat Support Battalions', color: '#ea580c' },
      { key: 'infantry', label: 'Infantry Battalions', color: '#16a34a' },
      { key: 'mobile', label: 'Mobile Battalions', color: '#d97706' },
      { key: 'mobile-cs', label: 'Mobile Combat Support Battalions', color: '#db2777' },
      { key: 'support', label: 'Support Companies', color: '#7c3aed' },
    ],
    rows: [
      {
        kind: 'data',
        key: 'GER',
        label: knownValue('German Reich'),
        segments: [
          { key: 'infantry', label: 'Infantry Battalions', share: 0.7, value: knownValue(280), color: '#16a34a' },
          { key: 'support', label: 'Support Companies', share: 0.3, value: knownValue(120), color: '#7c3aed' },
        ],
        readout: { primary: '400' },
      },
    ],
  };
  const layout = layoutFamilyBars(model, 840, 390);
  assert.ok(layout.legendHeight >= 44, 'six long category names wrap onto two legend rows');
  const { ctx, ops } = makeFakeCtx();
  drawFamilyBars(ctx, { model, layout, theme: THEME, scrollTop: 0, now: 0, glideDuration: 0 });
  const texts = ops.filter(op => op.op === 'fillText').map(op => String(op.args[0]));
  assert.ok(texts.includes('Armored Battalions'), 'legend includes first category');
  assert.ok(texts.includes('Support Companies'), 'wrapped legend includes last category');
  assert.ok(texts.includes('Battalions'), 'legend strip includes count unit once');
});

test('#833-7: all-zero integer Race axis emits whole ticks [0, 1] instead of repeated <1 labels', () => {
  const h = engineHour({ year: 1936, month: 1, day: 1, hour: 1 })!;
  const model: BarsModel = {
    form: 'ordered',
    hour: h,
    showAxis: true,
    scaleFormat: { decimals: 0 },
    rows: [
      { kind: 'data', key: 'GER', label: knownValue('German Reich'), value: { value: knownValue(0) }, numeric: 0 },
      { kind: 'data', key: 'FRA', label: knownValue('France'), value: { value: knownValue(0) }, numeric: 0 },
    ],
  };
  const layout = layoutFamilyBars(model, 600, 300);
  const { ctx, ops } = makeFakeCtx();
  drawFamilyBars(ctx, { model, layout, theme: THEME, scrollTop: 0, now: 0, glideDuration: 0 });
  const axisY = layout.axisTop + layout.axisHeight / 2;
  const axisTexts = ops.filter(op => op.op === 'fillText' && op.args[2] === axisY).map(op => String(op.args[0]));
  assert.deepEqual(axisTexts, ['0', '1']);

  const scaledModel: BarsModel = {
    form: 'ordered',
    hour: h,
    showAxis: true,
    scaleMax: 100,
    scaleFormat: { decimals: 2, maxDecimals: 0, unit: 'HP' },
    rows: [
      { kind: 'data', key: 'GER', label: knownValue('German Reich'), value: { value: knownValue(100) }, numeric: 100 },
    ],
  };
  const scaledLayout = layoutFamilyBars(scaledModel, 600, 300);
  const scaledCtx = makeFakeCtx();
  drawFamilyBars(scaledCtx.ctx, { model: scaledModel, layout: scaledLayout, theme: THEME, scrollTop: 0, now: 0, glideDuration: 0 });
  const scaledAxisY = scaledLayout.axisTop + scaledLayout.axisHeight / 2;
  const scaledTexts = scaledCtx.ops.filter(op => op.op === 'fillText' && op.args[2] === scaledAxisY).map(op => String(op.args[0]));
  assert.deepEqual(scaledTexts, ['0.00 HP', '0.25 HP', '0.50 HP', '0.75 HP', '1.00 HP']);

  const fixedPctModel: BarsModel = {
    form: 'ordered',
    hour: h,
    showAxis: true,
    scaleMax: 100_000,
    scaleFormat: 'fixed5Percent',
    rows: [
      { kind: 'data', key: 'GER', label: knownValue('German Reich'), value: { value: knownValue(100_000) }, numeric: 100_000 },
    ],
  };
  const fixedPctLayout = layoutFamilyBars(fixedPctModel, 600, 300);
  const fixedPctCtx = makeFakeCtx();
  drawFamilyBars(fixedPctCtx.ctx, { model: fixedPctModel, layout: fixedPctLayout, theme: THEME, scrollTop: 0, now: 0, glideDuration: 0 });
  const fixedPctAxisY = fixedPctLayout.axisTop + fixedPctLayout.axisHeight / 2;
  const fixedPctTexts = fixedPctCtx.ops.filter(op => op.op === 'fillText' && op.args[2] === fixedPctAxisY).map(op => String(op.args[0]));
  assert.deepEqual(fixedPctTexts, ['0.00%', '0.25%', '0.50%', '0.75%', '1.00%']);
});

test('C1.7: offscreen rank swaps and scrolling unchanged ranks in a 1,500-country Race trigger no glide motion', () => {
  const h = engineHour({ year: 1936, month: 1, day: 1, hour: 1 })!;
  const probeRows = Array.from({ length: 1500 }, (_, idx) => ({
    kind: 'data' as const, key: `C${idx}`, label: knownValue(`Country ${idx}`), value: { value: knownValue(1500 - idx) }, numeric: 1500 - idx,
  }));
  const offscreenStart = visibleBarsRange(layoutFamilyBars({ form: 'ordered', hour: h, rows: probeRows }, 480, 240), 0, 0).last;
  registerFamilyProjection<BarsProjection<Record<string, never>>>({
    kind: 'test-bars-offscreen-swap',
    family: 'bars',
    project(readView) {
      const rows = Array.from({ length: 1500 }, (_, idx) => {
        let keyIdx = idx;
        if (readView.hour >= h + 1) {
          if (idx === offscreenStart) keyIdx = offscreenStart + 1;
          else if (idx === offscreenStart + 1) keyIdx = offscreenStart;
        }
        if (readView.hour >= h + 3) {
          if (idx === 2) keyIdx = 3;
          else if (idx === 3) keyIdx = 2;
        }
        return { kind: 'data' as const, key: `C${keyIdx}`, label: knownValue(`Country ${keyIdx}`), value: { value: knownValue(1500 - idx) }, numeric: 1500 - idx };
      });
      return { form: 'ordered', hour: readView.hour, animateOrder: true, nextChangeAt: readView.hour + 1, rows };
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
    const view = createLaneView(barsLane, () => {});
    const state = new Map<string, unknown>();
    let seq = 0;
    const transport = new LaneTransport();
    const options: Record<string, never> = {};
    const sources = {};
    const bindingAt = (scrollTop: number) => transport.carry({
      family: 'bars', kind: 'test-bars-offscreen-swap', options, sources, width: 480, height: 240, scrollTop, theme: THEME,
    } satisfies FamilyBarsBinding<Record<string, never>>);
    const atTop = bindingAt(0);
    const scrolled = bindingAt(30);
    const input = (hour: number, now: number, model = atTop): PaintInput => ({
      widget: 1, subs: [], pictures: [], hour, seek: 0, speed: 24, now, inspect: null,
      binding: { generation: 1, surfaces: [{ width: 480, height: 240, dpr: 1 }], offset: 0, pin: null },
      model,
    });
    view.paint(input(h, 1000, atTop), state, () => `c${++seq}`);
    const afterOffscreenSwap = view.paint(input(h + 1, 1020, atTop), state, () => `c${++seq}`);
    assert.equal(afterOffscreenSwap.motion, false, 'swapping the first two fully offscreen rows triggers no glide motion');
    const afterScrollUnchanged = view.paint(input(h + 2, 1040, scrolled), state, () => `c${++seq}`);
    assert.equal(afterScrollUnchanged.motion, false, 'scrolling 30px when ranks did not change triggers no glide motion');
    const afterVisibleSwap = view.paint(input(h + 3, 1060, scrolled), state, () => `c${++seq}`);
    assert.equal(afterVisibleSwap.motion, true, 'swapping two visible rows still glides');
  } finally {
    (globalThis as { OffscreenCanvas: unknown }).OffscreenCanvas = origOffscreen;
  }
});

test('bars row labels carry Value<string> states through mirror, audit, tooltip, and muted painting', () => {
  const h = engineHour({ year: 1939, month: 1, day: 1, hour: 1 })!;
  const model: BarsModel = {
    form: 'ordered',
    hour: h,
    rows: [
      { kind: 'data', key: 'pending-row', label: pendingValue('Name is loading'), color: '#4f86f7', value: { value: knownValue(10), format: 'integer' }, numeric: 10 },
      { kind: 'data', key: 'literal-row', label: knownValue('Loading…'), color: '#3fb950', value: { value: knownValue(8), format: 'integer' }, numeric: 8 },
      { kind: 'data', key: 'missing-row', label: notRecorded('Country not recorded'), color: '#d29922', value: { value: knownValue(6), format: 'integer' }, numeric: 6 },
      {
        kind: 'data',
        key: 'custom-tip-row',
        label: pendingValue('Name is loading'),
        color: '#4f86f7',
        value: { value: knownValue(5), format: 'integer' },
        numeric: 5,
        tooltip: { title: 'Loading…', hour: h, items: [{ label: 'Factories', value: '5' }] },
      },
      {
        kind: 'data',
        key: 'readout-only-row',
        label: pendingValue('Name is loading'),
        readout: { primary: '7 mines' },
      },
      {
        kind: 'data',
        key: 'split-left',
        label: knownValue('Left'),
        value: { value: knownValue(3), format: 'integer' },
        numeric: 3,
        rightSide: { key: 'split-right', label: pendingValue('Name is loading'), value: { value: knownValue(4), format: 'integer' }, numeric: 4 },
      },
    ],
  };
  assertBarsModelValid(model, h);

  const mirror = deriveBarsMirror(model);
  assert.equal(mirror.rows[0]?.name, 'Loading: Name is loading');
  assert.equal(mirror.rows[1]?.name, 'Loading…');
  assert.equal(mirror.rows[2]?.name, 'Name not recorded: Country not recorded');

  const audit = deriveBarsAudit(model);
  const labelAudit = (key: string) => audit.find(entry => entry.key === `${key}:label`)?.value;
  assert.equal(labelAudit('pending-row'), '["pending","Name is loading"]');
  assert.equal(labelAudit('literal-row'), '["known",["str","Loading…"]]');
  assert.equal(labelAudit('missing-row'), '["notRecorded","Country not recorded"]');
  assert.equal(labelAudit('split-right'), '["pending","Name is loading"]');

  const defaultTip = deriveBarsTooltip(model, 'pending-row')!;
  assert.match(tooltipText(buildTooltipContent(defaultTip)), /Loading: Name is loading/);
  const customTip = deriveBarsTooltip(model, 'custom-tip-row')!;
  assert.match(tooltipText(buildTooltipContent(customTip)), /Loading: Name is loading/);
  const readoutTip = deriveBarsTooltip(model, 'readout-only-row')!;
  assert.deepEqual(readoutTip.items, [
    { label: 'Name', pendingReason: 'Name is loading' },
    { label: 'Count', value: '7 mines' },
  ]);

  const layout = layoutFamilyBars(model, 480, 220, THEME);
  const fake = makeFakeCtx();
  drawFamilyBars(fake.ctx, { model, layout, theme: THEME, scrollTop: 0, now: 0, glideDuration: 0 });
  const textOps = fake.ops.filter(op => op.op === 'fillText').map(op => String(op.args[0]));
  assert.ok(textOps.includes('Loading…'));
  assert.ok(textOps.includes('Name not recorded'));
});
