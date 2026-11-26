import assert from 'node:assert/strict';
import test from 'node:test';
import { paletteCache, readableColour } from '../charts/palette.ts';
import { clearTextWidthCache, measureCanvasText, textWidthCacheStats, type Canvas2D } from '../charts/textWidth.ts';
import { colorCache, mix, parseColorAlpha } from '../shell/appearance/color.ts';
import { engineHour, gameDate } from '../dashboard/calendar.ts';
import { markColours } from '../shell/appearance/themes.ts';
import type { FamilyTheme } from '../families/contract.ts';
import {
  BEHIND_BAR_HEIGHT,
  BEHIND_BAR_MAX_WIDTH,
  BEHIND_BAR_MIN_WIDTH,
  BEHIND_BAR_OFFSET,
  FLAG_LOSER_ALPHA,
  FLAG_SLOT_HEIGHT,
  FLAG_SLOT_WIDTH,
  artInitials,
  barCornerRadius,
  drawActionButton,
  drawArtSlot,
  drawBarBehindFigure,
  drawChevron,
  drawClockSummary,
  drawDateColumn,
  drawDateTab,
  drawFlagSlot,
  drawGapBand,
  drawGradientLegend,
  drawIconStrip,
  iconStripSlotWidth,
  drawLegendStrip,
  drawMissingMark,
  drawNameSlot,
  drawOutOfFigure,
  drawPartialTab,
  drawProgressRing,
  drawRowHairline,
  drawRowHover,
  drawRowSelected,
  drawSeriesMissingStrip,
  drawSkillFigure,
  drawSplitBar,
  drawStatusMark,
  drawNetBar,
  drawThresholdMeter,
  drawTrackBar,
  drawTwoWayBar,
  drawWinRateCell,
  resolveKitTokens,
  TIMELINE_LABEL_MIN_WIDTH,
  computeTimeAxisTicks,
  contrastInkForFill,
  fitLegendStrip,
  type LegendFit,
  type LegendMeasure,
  drawTimeAxis,
  drawTimelineBar,
  winRateColor,
} from './kit.ts';

const THEME: FamilyTheme = {
  surface: '#16181d',
  raised: '#1e2128',
  text: '#eef1f6',
  muted: '#8c95a6',
  line: '#2e3440',
  lineSoft: '#222630',
  accent: '#4f86f7',
  blue: '#4f86f7',
  red: '#e5534b',
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
  readonly alpha: number;
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
    beginPath() {},
    closePath() {},
    quadraticCurveTo() {},
    clip() {},
    moveTo(x: number, y: number) { ops.push({ op: 'moveTo', args: [x, y], alpha: this.globalAlpha, fillStyle: this.fillStyle, strokeStyle: this.strokeStyle }); },
    lineTo(x: number, y: number) { ops.push({ op: 'lineTo', args: [x, y], alpha: this.globalAlpha, fillStyle: this.fillStyle, strokeStyle: this.strokeStyle }); },
    stroke() { ops.push({ op: 'stroke', args: [], alpha: this.globalAlpha, fillStyle: this.fillStyle, strokeStyle: this.strokeStyle }); },
    fill() { ops.push({ op: 'fill', args: [], alpha: this.globalAlpha, fillStyle: this.fillStyle, strokeStyle: this.strokeStyle }); },
    fillRect(...args: number[]) { ops.push({ op: 'fillRect', args, alpha: this.globalAlpha, fillStyle: this.fillStyle, strokeStyle: this.strokeStyle }); },
    roundRect(...args: unknown[]) { ops.push({ op: 'roundRect', args, alpha: this.globalAlpha, fillStyle: this.fillStyle, strokeStyle: this.strokeStyle }); },
    arc(...args: number[]) { ops.push({ op: 'arc', args, alpha: this.globalAlpha, fillStyle: this.fillStyle, strokeStyle: this.strokeStyle }); },
    setLineDash(...args: unknown[]) { ops.push({ op: 'setLineDash', args, alpha: this.globalAlpha, fillStyle: this.fillStyle, strokeStyle: this.strokeStyle }); },
    drawImage(...args: unknown[]) { ops.push({ op: 'drawImage', args, alpha: this.globalAlpha, fillStyle: this.fillStyle, strokeStyle: this.strokeStyle }); },
    fillText(text: string, x: number, y: number) { ops.push({ op: 'fillText', args: [text, x, y], alpha: this.globalAlpha, fillStyle: this.fillStyle, strokeStyle: this.strokeStyle }); },
    measureText(text: string) {
      const px = Number.parseFloat(/\b(\d+(?:\.\d+)?)px/.exec(this.font)?.[1] ?? '12') || 12;
      return { width: [...new Intl.Segmenter(undefined, { granularity: 'grapheme' }).segment(text)].length * px * 0.55 };
    },
    createLinearGradient() { return { addColorStop() {} }; },
  };
  return { ctx: ctx as unknown as CanvasRenderingContext2D, ops };
}

export function slotForm(ops: readonly RecordedOp[]): 'empty' | 'image' | 'missing' | 'unrecorded' | 'refused' | 'failed' {
  if (!ops.length) return 'empty';
  if (ops.some(op => op.op === 'drawImage')) return 'image';
  const filled = ops.some(op => op.op === 'fill');
  if (filled && ops.some(op => op.op === 'arc')) return 'failed';
  if (filled && ops.some(op => op.op === 'lineTo')) return 'refused';
  return ops.some(op => op.op === 'fillText') ? 'missing' : 'unrecorded';
}

test('art and flag slots keep fixed geometry and draw one distinct form per status', () => {
  const bitmap = { width: 64, height: 32 } as unknown as CanvasImageSource;
  const states = [
    { state: { kind: 'none' as const }, form: 'empty' },
    { state: { kind: 'pending' as const }, form: 'empty' },
    { state: { kind: 'ready' as const, bitmap, width: 64, height: 32 }, form: 'image' },
    { state: { kind: 'missing' as const, reason: 'no layer holds it', label: 'Heavy Tank' }, form: 'missing' },
    { state: { kind: 'unrecorded' as const, reason: 'Portrait hour not recorded' }, form: 'unrecorded' },
    { state: { kind: 'refused' as const, reason: 'Unsupported format' }, form: 'refused' },
    { state: { kind: 'failed' as const, reason: 'Decode failed', retryable: true }, form: 'failed' },
  ] as const;

  for (const { state, form } of states) {
    for (const [name, draw] of [
      ['art', (ctx: CanvasRenderingContext2D) => drawArtSlot(ctx, 10, 20, 32, 32, state, THEME)],
      ['flag', (ctx: CanvasRenderingContext2D) => drawFlagSlot(ctx, 10, 20, state, THEME)],
    ] as const) {
      const { ctx, ops } = makeFakeCtx();
      const res = draw(ctx);
      assert.equal(res.drawn, state.kind, `${name} ${state.kind}`);
      assert.equal(slotForm(ops), form, `${name} ${state.kind} draws its own form`);
    }
  }

  const art = makeFakeCtx();
  assert.equal(drawArtSlot(art.ctx, 0, 0, 32, 32, { kind: 'missing', reason: 'no layer holds it', label: 'Heavy Tank' }, THEME).label, 'HT');
  const small = makeFakeCtx();
  assert.equal(drawArtSlot(small.ctx, 0, 0, 16, 16, { kind: 'missing', reason: 'no layer holds it' }, THEME).label, '?', 'a confirmed miss keeps its mark at small sizes');

  assert.equal(artInitials('Medium Tank Chassis'), 'MTC');
  assert.equal(artInitials(undefined), '');

  const flagPending = makeFakeCtx();
  const pendingFlag = drawFlagSlot(flagPending.ctx, 4, 8, { kind: 'pending' }, THEME);
  assert.deepEqual({ x: pendingFlag.x, y: pendingFlag.y, width: pendingFlag.width, height: pendingFlag.height }, { x: 4, y: 8, width: FLAG_SLOT_WIDTH, height: FLAG_SLOT_HEIGHT });
  assert.equal(flagPending.ops.length, 0);

  const flagMissing = makeFakeCtx();
  const missingFlag = drawFlagSlot(flagMissing.ctx, 4, 8, { kind: 'missing', reason: 'no layer holds it', label: 'ger' }, THEME, { dimmed: true });
  assert.equal(missingFlag.label, 'GER');
  assert.ok(flagMissing.ops.some(op => op.op === 'fillText' && op.args[0] === 'GER' && op.alpha === FLAG_LOSER_ALPHA));
});

test('bars use square start, rounded value end, 64px behind-figure cap, and 7px right offset', () => {
  assert.equal(barCornerRadius(6), 2);
  assert.equal(barCornerRadius(24), 4);

  const behind = makeFakeCtx();
  const rect = drawBarBehindFigure(behind.ctx, 120, 18, 1, '#4f86f7', { density: 'normal', sorted: true, theme: THEME });
  assert.ok(rect);
  assert.equal(rect.width, BEHIND_BAR_MAX_WIDTH);
  assert.equal(rect.height, BEHIND_BAR_HEIGHT.normal);
  assert.equal(rect.x + rect.width, 120 + BEHIND_BAR_OFFSET);
  const rr = behind.ops.find(op => op.op === 'roundRect');
  assert.ok(rr);
  assert.deepEqual(rr.args[4], [4, 0, 0, 4]);

  const twoWay = makeFakeCtx();
  drawTwoWayBar(twoWay.ctx, 100, 18, 40, 12, -0.5, '#3fb950', '#ff6b6f', THEME);
  const negRect = twoWay.ops.find(op => op.op === 'roundRect');
  assert.ok(negRect);
  assert.equal(negRect.args[0], 80);
  assert.equal(negRect.args[2], 20);
  assert.deepEqual(negRect.args[4], [4, 0, 0, 4]);

  const split = makeFakeCtx();
  drawSplitBar(split.ctx, 10, 10, 100, 12, [
    { fraction: 0.4, fill: '#4f86f7' },
    { fraction: 0.3, fill: '#3fb950' },
  ], THEME);
  assert.ok(split.ops.some(op => op.op === 'fillRect' && op.args[0] === 10 && op.args[2] === 40));
  const lastSeg = split.ops.find(op => op.op === 'roundRect');
  assert.ok(lastSeg);
  assert.equal(lastSeg.args[0], 50);
  assert.deepEqual(lastSeg.args[4], [0, 4, 4, 0]);

  const track = makeFakeCtx();
  drawTrackBar(track.ctx, 0, 18, 56, 6, 0.5, '#4f86f7', THEME);
  const rects = track.ops.filter(op => op.op === 'roundRect');
  assert.equal(rects.length, 2);
  assert.equal(rects[1]!.args[2], 28);
});

test('name slots report truncation, status marks use dots/rings, and date/missing/chart primitives honour approved specs', () => {
  const nameCtx = makeFakeCtx();
  const fitted = drawNameSlot(nameCtx.ctx, 'Very Long Country Or Division Name That Overflows', 0, 0, 70, 28, THEME);
  assert.equal(fitted.truncated, true);
  assert.equal(fitted.lines.length, 1);
  assert.equal(fitted.full, 'Very Long Country Or Division Name That Overflows');

  const statusCtx = makeFakeCtx();
  const running = drawStatusMark(statusCtx.ctx, 0, 18, 'running', THEME);
  const waiting = drawStatusMark(statusCtx.ctx, 80, 18, 'waiting', THEME);
  assert.equal(running.label.full, 'Running');
  assert.equal(waiting.label.full, 'Waiting');
  assert.ok(statusCtx.ops.some(op => op.op === 'fill'));
  assert.ok(statusCtx.ops.some(op => op.op === 'stroke'));

  const hour = engineHour({ year: 1941, month: 9, day: 14, hour: 15 })!;
  const dateCtx = makeFakeCtx();
  const dateDrawn = drawDateColumn(dateCtx.ctx, 0, 0, 36, hour, THEME);
  assert.equal(dateDrawn.missing, false);
  assert.equal(dateDrawn.full, '14 Sep 1941');
  assert.equal(dateDrawn.hover, '14 Sep 1941 · 15:00');
  const drawnTexts = dateCtx.ops.filter(op => op.op === 'fillText').map(op => op.args[0]);
  assert.deepEqual(drawnTexts, ['14 Sep', '1941']);

  const missingDateCtx = makeFakeCtx();
  const missingDate = drawDateColumn(missingDateCtx.ctx, 0, 0, 36, null, THEME);
  assert.equal(missingDate.missing, true);
  assert.equal(missingDate.hover, 'Date not recorded');

  const narrowMissing = makeFakeCtx();
  const sq = drawMissingMark(narrowMissing.ctx, 0, 18, 24, THEME, { figure: 'Fuel' });
  assert.equal(sq.squareOnly, true);
  assert.equal(sq.label, 'Fuel not recorded');

  const windowMissing = makeFakeCtx();
  const na = drawMissingMark(windowMissing.ctx, 0, 18, 40, THEME, { gameWindow: true });
  assert.equal(na.label, 'n/a');
  assert.ok(windowMissing.ops.some(op => op.op === 'fillText' && op.args[0] === 'n/a'));

  const tabCtx = makeFakeCtx();
  assert.deepEqual(drawPartialTab(tabCtx.ctx, 10, 18, THEME), { width: 6, height: 13 });

  const chromeCtx = makeFakeCtx();
  drawRowHover(chromeCtx.ctx, 0, 0, 300, 36, THEME);
  drawRowSelected(chromeCtx.ctx, 0, 36, 300, 36, THEME);
  drawRowHairline(chromeCtx.ctx, 0, 300, 36, THEME, 'band');
  drawGapBand(chromeCtx.ctx, 20, 0, 40, 120, THEME);
  drawSeriesMissingStrip(chromeCtx.ctx, 20, 115, 40, '#4f86f7', THEME);
  const tab = drawDateTab(chromeCtx.ctx, '14 Sep 1941', 5, 120, THEME, 'playhead', { left: 10, right: 200 });
  assert.equal(tab.x, 10);
  const grad = drawGradientLegend(chromeCtx.ctx, 20, 14, '-100', '+100', ['#d95926', '#444444', '#199e70'], THEME);
  assert.ok(grad.width > 64);
  const stripCtx = makeFakeCtx();
  drawLegendStrip(stripCtx.ctx, 0, 0, 500, 26, {
    gradient: { lowLabel: '0%', highLabel: '100%', stops: ['#d95926', '#199e70'] },
    entries: [
      { label: 'Not recorded', hatch: true },
      { label: 'Unobserved', outlined: true },
      { label: 'Limit', color: '#4f86f7', dash: [4, 2] },
    ],
    rightNote: 'As of 1 Jan 1940',
    padX: 12,
    hairline: true,
  }, THEME);
  const stripTexts = stripCtx.ops.filter(op => op.op === 'fillText').map(op => op.args[0]);
  assert.ok(stripTexts.includes('Not recorded'));
  assert.ok(stripTexts.includes('Unobserved'));
  assert.ok(stripTexts.includes('Limit'));
  assert.ok(stripTexts.includes('As of 1 Jan 1940'));
  const hoverTab = drawDateTab(chromeCtx.ctx, '1 Sep 1939', 100, 120, THEME, 'hover', { left: 10, right: 200 });
  assert.ok(hoverTab.width > 0);
  const tokens = resolveKitTokens(THEME);
  assert.equal(tokens.shortage, readableColour(markColours(THEME.red, THEME.green).shortInk, THEME.surface), 'negative figures use the ink derived from the theme error role');
  assert.equal(tokens.dateTabBg, THEME.raised);
  assert.equal(tokens.hoverTabBg, THEME.text);

  const meterCtx = makeFakeCtx();
  drawThresholdMeter(meterCtx.ctx, 10, 18, 100, 8, 0.5, [
    { at: 0.25, reached: true },
    { at: 0.5, reached: true },
    { at: 0.75, reached: false },
    { at: 1.0, reached: false },
  ], '#4f86f7', THEME);
  assert.equal(meterCtx.ops.filter(op => op.op === 'moveTo').length, 3);
  const missingMeter = makeFakeCtx();
  drawThresholdMeter(missingMeter.ctx, 10, 18, 100, 8, null, [{ at: 0.5, reached: false }], '#4f86f7', THEME);
  assert.equal(missingMeter.ops.filter(op => op.op === 'moveTo').length > 0, true);

  const ringCtx = makeFakeCtx();
  drawProgressRing(ringCtx.ctx, 12, 12, 6.2, 0.75, '#d0607c', THEME);
  assert.equal(ringCtx.ops.filter(op => op.op === 'arc').length, 2);
});

test('timeline bars hide icon and name under the minimum width, mix ended items toward the surface, and the axis labels years or months', () => {
  const bitmap = { width: 32, height: 32 } as unknown as CanvasImageSource;
  const icon = { kind: 'ready', bitmap, width: 32, height: 32 } as const;
  const narrow = makeFakeCtx();
  const narrowRes = drawTimelineBar(narrow.ctx, { x: 10, y: 10, width: TIMELINE_LABEL_MIN_WIDTH - 1, height: 18, fill: '#3987e5', theme: THEME, open: true, icon, label: 'Free Trade' });
  assert.deepEqual([narrowRes.showedIcon, narrowRes.label], [false, null]);

  const wide = makeFakeCtx();
  const wideRes = drawTimelineBar(wide.ctx, { x: 10, y: 10, width: 140, height: 18, fill: '#3987e5', theme: THEME, heldBefore: true, open: true, icon, label: 'Free Trade' });
  assert.equal(wideRes.showedIcon, true);
  assert.equal(wideRes.label?.text, 'Free Trade');
  assert.ok(wide.ops.some(op => op.op === 'drawImage'));

  const open = makeFakeCtx();
  drawTimelineBar(open.ctx, { x: 0, y: 0, width: 40, height: 14, fill: '#3987e5', theme: THEME, open: true });
  const ended = makeFakeCtx();
  drawTimelineBar(ended.ctx, { x: 0, y: 0, width: 40, height: 14, fill: '#3987e5', theme: THEME, open: false });
  const fillOf = (ops: ReturnType<typeof makeFakeCtx>['ops']) => ops.find(op => op.op === 'fill')?.fillStyle;
  assert.notEqual(fillOf(open.ops), fillOf(ended.ops), 'an ended item is mixed toward the surface');

  const h1936 = engineHour({ year: 1936, month: 1, day: 1, hour: 1 })!;
  const h1941 = engineHour({ year: 1941, month: 9, day: 15, hour: 1 })!;
  const years = computeTimeAxisTicks(h1936, h1941, 600);
  assert.deepEqual(years.map(tick => tick.label), ['1936', '1937', '1938', '1939', '1940', '1941']);
  const h1937 = engineHour({ year: 1937, month: 1, day: 1, hour: 1 })!;
  const underTwoYears = computeTimeAxisTicks(h1936, h1937, 600);
  assert.deepEqual(
    underTwoYears.filter(tick => tick.label).map(tick => tick.label),
    ['1936', 'Apr', 'Jul', 'Oct', '1937'],
    'under two years labels Jan with year and Apr/Jul/Oct as quarters',
  );
  assert.equal(underTwoYears.length, 13, 'under two years emits 3px ticks for every month');
  const hMar37 = engineHour({ year: 1937, month: 3, day: 1, hour: 1 })!;
  const hDec37 = engineHour({ year: 1937, month: 12, day: 20, hour: 1 })!;
  const noJanTicks = computeTimeAxisTicks(hMar37, hDec37, 600);
  assert.deepEqual(
    noJanTicks.filter(tick => tick.label).map(tick => tick.label),
    ['Apr 1937', 'Jul', 'Oct'],
    'without a January in view the first labelled month carries its year',
  );
  const hWeek = engineHour({ year: 1936, month: 1, day: 8, hour: 1 })!;
  const dayTicks = computeTimeAxisTicks(h1936, hWeek, 600);
  assert.ok(dayTicks.some(tick => tick.label === '4 Jan'), 'short windows show days');
  assert.ok(computeTimeAxisTicks(0, 167, 600, 24).length >= 2, 'non-calendar short windows still yield day ticks');
  const hJan10 = engineHour({ year: 1937, month: 1, day: 10, hour: 1 })!;
  const hJun37 = engineHour({ year: 1937, month: 6, day: 1, hour: 1 })!;
  const fromTenth = computeTimeAxisTicks(hJan10, hJun37, 600);
  assert.equal(fromTenth[0]!.hour, engineHour({ year: 1937, month: 2, day: 1, hour: 1 }), 'R2-K9: a mark sits at a calendar boundary, never moved to the window start');
  assert.deepEqual(fromTenth.filter(tick => tick.label).map(tick => tick.label), ['Apr 1937']);
  const oneDay = computeTimeAxisTicks(h1936, h1936 + 24, 600);
  assert.deepEqual([oneDay[0]?.hour, oneDay[0]?.label, oneDay[0]?.major], [h1936, '1936', true], 'R2-K9: a one-day window starting on 1 January keeps its tick');
  const hJan3 = engineHour({ year: 1936, month: 1, day: 3, hour: 1 })!;
  const week = computeTimeAxisTicks(hJan3, hJan3 + 7 * 24, 600);
  assert.equal(week.find(tick => tick.label)?.label, '3 Jan 1936', 'R2-K9: the first labelled day carries its year');
  const h1946 = engineHour({ year: 1946, month: 6, day: 1, hour: 1 })!;
  const decade = computeTimeAxisTicks(h1936, h1946, 200);
  assert.equal(decade.length, 11, 'R2-K9: every 1 January is marked in a narrow decade');
  assert.ok(decade.every(tick => tick.major && gameDate(tick.hour)?.month === 1 && gameDate(tick.hour)?.day === 1));
  assert.deepEqual(decade.filter(tick => tick.label).map(tick => tick.label), ['1940', '1945'], 'labels are thinned apart from the marks');
  const hugeNonCalendar = computeTimeAxisTicks(0, 43_800_000, 600, 24);
  assert.ok(hugeNonCalendar.length >= 2 && hugeNonCalendar.length <= 20, 'K10: huge non-calendar span stays bounded');

  const axis = makeFakeCtx();
  drawTimeAxis(axis.ctx, years, { plotLeft: 100, plotRight: 700, axisTop: 200, startHour: h1936, endHour: h1941, playhead: h1941, playheadLabel: '15 Sep 1941', theme: THEME });
  const texts = axis.ops.filter(op => op.op === 'fillText').map(op => op.args[0]);
  assert.ok(texts.includes('15 Sep 1941'));
  assert.ok(!texts.includes('1941'), 'a year label under the playhead tab is dropped');
  assert.ok(axis.ops.some(op => op.op === 'lineTo' && op.args[1] === 205), 'major January ticks are 5px tall');
});

test('table cell kit primitives handle minimum bar width, win rate tinting, icon strips, actions, stockpile bars and clock summary', () => {
  const tiny = makeFakeCtx();
  const tinyRect = drawBarBehindFigure(tiny.ctx, 120, 18, 0.01, '#4f86f7', { density: 'normal', sorted: true, theme: THEME });
  assert.ok(tinyRect);
  assert.equal(tinyRect.width, BEHIND_BAR_MIN_WIDTH);

  const chev = makeFakeCtx();
  drawChevron(chev.ctx, 4, 18, false, THEME);
  drawChevron(chev.ctx, 4, 18, true, THEME);
  assert.equal(chev.ops.filter(op => op.op === 'stroke').length, 2);

  const rateTokens = resolveKitTokens(THEME);
  assert.deepEqual([winRateColor(0.2, THEME), winRateColor(0.5, THEME), winRateColor(0.8, THEME)], [rateTokens.shortage, rateTokens.textSecondary, rateTokens.goodInk]);
  const wr = makeFakeCtx();
  assert.deepEqual(drawWinRateCell(wr.ctx, 0, 120, 18, 0, 0, THEME), { pct: null });
  assert.deepEqual(drawWinRateCell(wr.ctx, 0, 120, 18, 12, 4, THEME), { pct: 75 });

  const outOf = makeFakeCtx();
  drawOutOfFigure(outOf.ctx, '12', '15', 0, 100, 18, THEME);
  assert.ok(outOf.ops.some(op => op.op === 'fillText' && op.args[0] === ' / 15'));

  const skill = makeFakeCtx();
  drawSkillFigure(skill.ctx, 0, 80, 18, '5', { kind: 'missing', reason: 'no layer holds it', label: 'Attack' }, THEME);
  assert.ok(skill.ops.some(op => op.op === 'fillText' && op.args[0] === '5'));

  const strip = makeFakeCtx();
  const res = drawIconStrip(strip.ctx, 0, 90, 18, [
    { state: { kind: 'missing', reason: 'no layer holds it', label: 'A' }, name: 'A' },
    { state: { kind: 'missing', reason: 'no layer holds it', label: 'B' }, name: 'B' },
    { state: { kind: 'missing', reason: 'no layer holds it', label: 'C' }, name: 'C' },
    { state: { kind: 'missing', reason: 'no layer holds it', label: 'D' }, name: 'D' },
  ], THEME, { kind: 'traits' });
  assert.equal(res.drawnCount + res.overflowCount, 4);
  assert.ok(res.overflowCount > 0);
  assert.ok(strip.ops.some(op => op.op === 'fillText' && op.args[0] === `+${res.overflowCount}`), 'a strip that overflows shows +N in the last slot');

  const fitStrip = makeFakeCtx();
  const fitRes = drawIconStrip(fitStrip.ctx, 0, 200, 18, [
    { state: { kind: 'missing', reason: 'no layer holds it', label: 'A' }, name: 'A' },
    { state: { kind: 'missing', reason: 'no layer holds it', label: 'B' }, name: 'B' },
  ], THEME, { kind: 'traits' });
  assert.deepEqual(fitRes, { drawnCount: 2, overflowCount: 0 });
  assert.ok(!fitStrip.ops.some(op => op.op === 'fillText' && String(op.args[0]).startsWith('+')));

  const netShort = makeFakeCtx();
  drawNetBar(netShort.ctx, 0, 120, 0, 36, -0.5, THEME);
  assert.equal(netShort.ops.find(op => op.op === 'fill')?.fillStyle, resolveKitTokens(THEME).shortFill, 'a shortage net bar is filled with the fill derived from the theme error role');
  const tall = { kind: 'ready', bitmap: {} as CanvasImageSource, width: 18, height: 36 } as const;
  assert.deepEqual([iconStripSlotWidth(tall, 28), iconStripSlotWidth({ ...tall, width: 72 }, 28), iconStripSlotWidth({ kind: 'pending' }, 28)], [14, 28, 28],
    'a tall medal takes its own width at the strip height, so the gap between medals is the strip gap');

  const act = makeFakeCtx();
  const restBox = drawActionButton(act.ctx, 100, 18, 'View', false, THEME);
  const hoverBox = drawActionButton(act.ctx, 100, 18, 'View', true, THEME);
  assert.ok(hoverBox.width > restBox.width);
  assert.ok(act.ops.some(op => op.op === 'fillText' && op.args[0] === 'View'));

  const stock = makeFakeCtx();
  drawNetBar(stock.ctx, 0, 200, 0, 48, -0.25, THEME);
  const short = stock.ops.find(op => op.op === 'roundRect');
  assert.deepEqual(short?.args.slice(0, 4), [100.5 - 25, 18, 25, 12], 'a quarter short of need draws a quarter of the half width, left of the centre, 12 px tall');
  const over = makeFakeCtx();
  drawNetBar(over.ctx, 0, 200, 0, 48, 3, THEME);
  assert.equal(over.ops.find(op => op.op === 'roundRect')?.args[2], 98, 'a surplus is capped at the whole half width');
  const none = makeFakeCtx();
  drawNetBar(none.ctx, 0, 200, 0, 48, null, THEME);
  assert.equal(none.ops.some(op => op.op === 'roundRect'), false, 'no share, no bar; the centre line stays');

  const clock = makeFakeCtx();
  drawClockSummary(clock.ctx, 320, 180, {
    totalText: '12:30:00',
    playingText: '10:00:00',
    pausedText: '2:30:00',
    playingFraction: 0.8,
    pausedFraction: 0.2,
    playingPct: '80%',
    pausedPct: '20%',
    complete: true,
    coverageLabel: 'Complete',
    spanText: '1 Jan 1936 – 1 Jan 1940',
  }, THEME);
  assert.ok(clock.ops.some(op => op.op === 'fillText' && op.args[0] === '12:30:00'));

  const fillInk = resolveKitTokens(THEME);
  assert.equal(contrastInkForFill('#f4f6f9', THEME), fillInk.darkFillInk);
  assert.equal(contrastInkForFill('#888888', THEME), fillInk.darkFillInk);
  assert.equal(contrastInkForFill('#7a7a7a', THEME), fillInk.lightFillInk, 'R2-K8: the inks are compared as painted, translucent over the fill');
  assert.equal(contrastInkForFill('#1e4fa3', THEME), fillInk.lightFillInk);
  assert.equal(contrastInkForFill('#16181d', THEME), fillInk.lightFillInk);
  assert.equal(contrastInkForFill('not-a-colour', THEME), THEME.text);
});

test('K6 & K8: legend strip draws whole entries with +N more overflow, keeps missing key, unifies swatch geometry, and reads lazily', () => {
  const entries = [
    { label: 'Oil', color: '#c87d4b' },
    { label: 'Aluminium', color: '#7fa6d6' },
    { label: 'Rubber', color: '#64a86e' },
    { label: 'Tungsten', color: '#a580d2' },
    { label: 'Steel', color: '#9aa4b2' },
    { label: 'Chromium', color: '#d49a4c' },
  ];
  const narrow = makeFakeCtx();
  drawLegendStrip(narrow.ctx, 0, 0, 260, 22, { entries, padX: 12, entryGap: 12 }, THEME);
  const narrowTexts = narrow.ops.filter(op => op.op === 'fillText').map(op => String(op.args[0]));
  assert.ok(narrowTexts.every(t => !t.includes('…')), 'overflowing entries are never ellipsised');
  const moreTag = narrowTexts.find(t => /^\+\d+ more$/.test(t));
  assert.ok(moreTag, `expected +N more overflow tag, got ${JSON.stringify(narrowTexts)}`);
  const hiddenCount = Number(/^\+(\d+) more$/.exec(moreTag!)![1]);
  const shownOrdinary = narrowTexts.filter(t => t !== moreTag);
  assert.equal(shownOrdinary.length + hiddenCount, entries.length, 'shown + hidden equals total ordinary entries');
  assert.ok(shownOrdinary.length >= 1, 'at least one whole entry fits');

  const withMissing = makeFakeCtx();
  drawLegendStrip(withMissing.ctx, 0, 0, 260, 22, {
    entries: [...entries.slice(0, 4), { label: 'Not recorded', hatch: true, muted: true }],
    padX: 12,
    entryGap: 12,
  }, THEME);
  const missingTexts = withMissing.ops.filter(op => op.op === 'fillText').map(op => String(op.args[0]));
  assert.ok(missingTexts.includes('Not recorded'), 'missing key is kept in preference to ordinary entries');
  assert.ok(missingTexts.some(t => /^\+\d+ more$/.test(t)), 'dropped ordinary entries are counted in +N more');

  const fillSwatch = makeFakeCtx();
  drawLegendStrip(fillSwatch.ctx, 0, 0, 200, 22, { entries: [{ label: 'Steel', color: '#4f86f7' }] }, THEME);
  assert.ok(
    fillSwatch.ops.some(op => op.op === 'roundRect' && op.args[2] === 9 && op.args[3] === 9),
    'fill swatch uses a 9x9 filled square',
  );

  const lineSwatch = makeFakeCtx();
  drawLegendStrip(lineSwatch.ctx, 0, 0, 200, 22, { entries: [{ label: 'Germany', color: '#4f86f7', line: true }] }, THEME);
  const lineMove = lineSwatch.ops.find(op => op.op === 'moveTo');
  const lineTo = lineSwatch.ops.find(op => op.op === 'lineTo');
  assert.ok(lineMove && lineTo && Number(lineTo.args[0]) - Number(lineMove.args[0]) === 14, 'line swatch draws a 14px line');

  let reads = 0;
  const lazy = makeFakeCtx();
  drawLegendStrip(lazy.ctx, 0, 0, 320, 22, {
    entries: {
      length: 1500,
      at(i: number) {
        reads++;
        return { label: `Part ${i}`, color: '#4f86f7' };
      },
      trailing: [{ label: 'Not recorded', hatch: true, muted: true }],
    },
    padX: 12,
    entryGap: 12,
  }, THEME);
  assert.ok(reads < 20, `K8: lazy legend should stop at the visible edge, read ${reads} of 1500`);
  const lazyTexts = lazy.ops.filter(op => op.op === 'fillText').map(op => String(op.args[0]));
  assert.ok(lazyTexts.includes('Not recorded'), 'trailing missing key kept in lazy source');
  assert.ok(lazyTexts.some(t => /^\+\d+ more$/.test(t)), 'lazy overflow reports exact +N more count');

  const chars: LegendMeasure = text => text.length * 6;
  const summary = (fit: LegendFit) => [fit.shown.map(entry => entry.label), fit.hidden, fit.missing.map(entry => entry.label)];
  const long = { label: 'The Union of Soviet Socialist Republics', color: '#4f86f7' };
  assert.deepEqual(summary(fitLegendStrip(0, 120, { entries: [long] }, THEME, chars)), [[], 1, []], 'R2-K11: a sole entry that does not fit whole is counted, never ellipsised or dropped');
  assert.deepEqual(summary(fitLegendStrip(0, 400, { entries: [long] }, THEME, chars)), [[long.label], 0, []]);
  const oversizedMissing = { label: 'Not recorded for most of this recording', hatch: true };
  assert.deepEqual(summary(fitLegendStrip(0, 120, { entries: [oversizedMissing] }, THEME, chars)), [[], 1, []], 'R2-K11: a lone oversized missing key is counted');
  const gradient = { lowLabel: 'Low', highLabel: 'High', stops: ['#000000', '#ffffff'] };
  const withGradient = fitLegendStrip(0, 345, { gradient, entries: [{ label: 'Germany', color: '#4f86f7' }, { label: 'Italy', color: '#64a86e' }, { label: 'Not recorded', hatch: true }] }, THEME, chars);
  assert.equal(withGradient.entriesLeft, 3 * 6 + 6 + 64 + 6 + 4 * 6 + 14, 'the gradient is deducted once');
  assert.deepEqual(summary(withGradient), [['Germany'], 1, ['Not recorded']], 'R2-K11: one fit from the actual cursor keeps the count and the missing key');
  const one = [{ label: 'Germany', color: '#4f86f7' }, { label: 'Italy', color: '#64a86e' }];
  const tight = fitLegendStrip(0, 15 + 7 * 6 + 12 + 15 + 5 * 6, { entries: one }, THEME, chars);
  assert.deepEqual(summary(tight), [['Germany', 'Italy'], 0, []], '"+1 more" is never shown where the entry itself fits');
});

test('K8 & K9: kit tokens cache per theme, 1,000 cells of 5 colours convert 5 colours, and text measurer stays bounded without native font reads on hits', () => {
  const first = resolveKitTokens(THEME);
  const second = resolveKitTokens(THEME);
  assert.equal(first, second, 'K8: resolveKitTokens caches tokens per theme');
  const lightTheme: FamilyTheme = { ...THEME, surface: '#f5f6f8', raised: '#ffffff', text: '#111111', dark: false };
  assert.notEqual(resolveKitTokens(lightTheme), first, 'theme change yields fresh tokens');

  const five = ['#2a4b8c', '#8c3b2a', '#2a7c4b', '#7c6b2a', '#6b2a7c'];
  const freshTheme: FamilyTheme = { ...THEME };
  resolveKitTokens(freshTheme);
  colorCache.clear();
  paletteCache.clear();
  let afterFirstFiveParses = 0;
  let afterFirstFiveOklch = 0;
  for (let i = 0; i < 1000; i++) {
    const raw = five[i % five.length]!;
    const lifted = readableColour(raw, freshTheme.surface);
    const mixed = mix(freshTheme.surface, lifted, 0.5);
    contrastInkForFill(mixed, freshTheme);
    if (i === 4) {
      afterFirstFiveParses = colorCache.parses;
      afterFirstFiveOklch = colorCache.oklchToRgb;
    }
  }
  assert.equal(paletteCache.readableSolves, 5, 'K8: 1,000 cells of 5 colours solve readableColour 5 times');
  assert.equal(colorCache.mixes, 5, 'K8: 1,000 cells of 5 colours compute mix 5 times');
  assert.equal(colorCache.parses, afterFirstFiveParses, 'K8: cells 6..1000 parse 0 colour strings');
  assert.equal(colorCache.oklchToRgb, afterFirstFiveOklch, 'K8: cells 6..1000 run 0 oklchToRgb conversions');
  resolveKitTokens(lightTheme);
  assert.equal(paletteCache.readableSolves, 0, 'K8: theme surface change clears colour and palette caches');

  clearTextWidthCache();
  let fontReads = 0;
  let measureCalls = 0;
  let currentFont = '400 12px Inter, sans-serif';
  const ctx = {
    get font() { fontReads++; return currentFont; },
    set font(v: string) { currentFont = v; },
    measureText(text: string) { measureCalls++; return { width: text.length * 6 }; },
  } as unknown as Canvas2D;
  measureCanvasText(ctx, 'German Reich', '500 12px Inter, sans-serif');
  const readsAfterWarm = fontReads;
  const callsAfterWarm = measureCalls;
  for (let i = 0; i < 100; i++) {
    measureCanvasText(ctx, 'German Reich', '500 12px Inter, sans-serif');
  }
  assert.equal(measureCalls, callsAfterWarm, 'cache hit never calls measureText');
  assert.equal(fontReads, readsAfterWarm, 'K9: cache hit never reads native ctx.font');

  for (let i = 0; i < 100; i++) {
    measureCanvasText(ctx, 'x', `400 ${10 + i}px Font${i}, sans-serif`);
  }
  assert.ok(textWidthCacheStats().fonts <= 64, `K9: outer font map stays bounded (got ${textWidthCacheStats().fonts})`);

  resolveKitTokens(lightTheme);
  readableColour('#2a4b8c', lightTheme.surface);
  for (let i = 0; i < 20; i++) parseColorAlpha(`#1${String(i).padStart(5, '0')}`);
  const parsesBefore = colorCache.parses;
  assert.ok(textWidthCacheStats().fonts > 0 && paletteCache.readableSolves > 0);
  resolveKitTokens({ ...lightTheme, text: '#222222', font: 'Other Sans' });
  assert.deepEqual([textWidthCacheStats().fonts, paletteCache.readableSolves], [0, 2],
    'R2-K13: a theme change on the same surface clears the bounded text and palette tables; only the new theme refills them (its two mark inks)');
  assert.ok(colorCache.parses < parsesBefore, 'and the colour tables, which only the new theme refills');

  const mutable: { -readonly [K in keyof FamilyTheme]: FamilyTheme[K] } = { ...THEME, rowHover: '\u0001', gridLine: '#223344' };
  const initialTokens = resolveKitTokens(mutable);
  assert.equal(initialTokens.rowHover, '\u0001');
  assert.equal(initialTokens.gridLine, '#223344');
  readableColour('#2a4b8c', mutable.surface);
  assert.ok(paletteCache.readableSolves > 2);

  delete mutable.rowHover;
  const deletedTokens = resolveKitTokens(mutable);
  assert.notEqual(deletedTokens, initialTokens, 'R3-S13: deleting a token-only override invalidates cached tokens without sentinel collision');
  assert.equal(deletedTokens.rowHover, resolveKitTokens({ ...THEME }).rowHover);
  resolveKitTokens(mutable);
  readableColour('#2a4b8c', mutable.surface);
  readableColour('#8c3b2a', mutable.surface);
  readableColour('#2a7c4b', mutable.surface);
  assert.ok(paletteCache.readableSolves >= 3);

  mutable.rowHover = '#445566';
  const mutatedTokens = resolveKitTokens(mutable);
  assert.notEqual(mutatedTokens, deletedTokens, 'R3-S13: mutating a token-only override invalidates the cached tokens');
  assert.equal(mutatedTokens.rowHover, '#445566');
  assert.equal(paletteCache.readableSolves, 2, 'R3-S13: changing a token-only override clears shared caches');

  readableColour('#2a4b8c', mutable.surface);
  assert.ok(paletteCache.readableSolves > 2);
  const clonedOverride: FamilyTheme = { ...mutable, gridLine: '#556677', hoverTabBg: '#778899' };
  const clonedTokens = resolveKitTokens(clonedOverride);
  assert.equal(clonedTokens.gridLine, '#556677');
  assert.equal(clonedTokens.hoverTabBg, '#778899');
  assert.equal(paletteCache.readableSolves, 2, 'R3-S13: cloned theme changing only token overrides clears shared caches');
  assert.equal(resolveKitTokens(clonedOverride), clonedTokens, 'unchanged theme with overrides still hits cache');
});
