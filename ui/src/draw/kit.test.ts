import assert from 'node:assert/strict';
import test from 'node:test';
import { engineHour } from '../dashboard/calendar.ts';
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
  drawStockpileSumCell,
  drawThresholdMeter,
  drawTrackBar,
  drawTwoWayBar,
  drawWinRateCell,
  resolveKitTokens,
  TIMELINE_LABEL_MIN_WIDTH,
  computeTimeAxisTicks,
  contrastInkForFill,
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

test('art and flag slots keep fixed geometry across every state and never hatch pending slots', () => {
  const bitmap = { width: 64, height: 32 } as unknown as CanvasImageSource;
  const states = [
    { kind: 'none' as const },
    { kind: 'pending' as const },
    { kind: 'ready' as const, bitmap, width: 64, height: 32 },
    { kind: 'missing' as const, resolvedName: 'Heavy Tank' },
    { kind: 'refused' as const, reason: 'Unsupported format' },
    { kind: 'failed' as const, reason: 'Decode failed' },
  ];

  for (const state of states) {
    const { ctx, ops } = makeFakeCtx();
    const res = drawArtSlot(ctx, 10, 20, 32, 32, state, THEME);
    assert.deepEqual({ x: res.x, y: res.y, width: res.width, height: res.height }, { x: 10, y: 20, width: 32, height: 32 });
    assert.equal(res.drawn, state.kind);
    if (state.kind === 'none' || state.kind === 'pending') {
      assert.equal(ops.length, 0, `${state.kind} slot must draw nothing`);
    }
    if (state.kind === 'missing') {
      assert.equal(res.initials, 'HT');
      assert.ok(ops.some(op => op.op === 'fillText' && op.args[0] === 'HT'));
    }
    if (state.kind === 'refused' || state.kind === 'failed') {
      assert.ok(ops.some(op => op.op === 'arc'));
      assert.equal(ops.some(op => op.op === 'fillText'), false);
    }
  }

  const smallMissing = makeFakeCtx();
  const smallRes = drawArtSlot(smallMissing.ctx, 0, 0, 24, 24, { kind: 'missing', resolvedName: 'Heavy Tank' }, THEME);
  assert.equal(smallRes.initials, undefined);
  assert.equal(smallMissing.ops.some(op => op.op === 'fillText'), false);

  assert.equal(artInitials('Medium Tank Chassis'), 'MTC');
  assert.equal(artInitials(undefined), '');

  const flagPending = makeFakeCtx();
  const pendingFlag = drawFlagSlot(flagPending.ctx, 4, 8, { kind: 'pending' }, THEME);
  assert.deepEqual({ x: pendingFlag.x, y: pendingFlag.y, width: pendingFlag.width, height: pendingFlag.height }, { x: 4, y: 8, width: FLAG_SLOT_WIDTH, height: FLAG_SLOT_HEIGHT });
  assert.equal(flagPending.ops.length, 0);

  const flagMissing = makeFakeCtx();
  const missingFlag = drawFlagSlot(flagMissing.ctx, 4, 8, { kind: 'missing', tag: 'ger' }, THEME, { dimmed: true });
  assert.equal(missingFlag.tag, 'GER');
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
  assert.equal(tokens.shortage, '#ff6b6f');
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

  assert.equal(winRateColor(0.2, THEME), 'rgb(255, 107, 111)');
  assert.equal(winRateColor(0.5, THEME), 'rgb(212, 212, 212)');
  assert.equal(winRateColor(0.8, THEME), 'rgb(111, 212, 150)');
  const wr = makeFakeCtx();
  assert.deepEqual(drawWinRateCell(wr.ctx, 0, 120, 18, 0, 0, THEME), { pct: null });
  assert.deepEqual(drawWinRateCell(wr.ctx, 0, 120, 18, 12, 4, THEME), { pct: 75 });

  const outOf = makeFakeCtx();
  drawOutOfFigure(outOf.ctx, '12', '15', 0, 100, 18, THEME);
  assert.ok(outOf.ops.some(op => op.op === 'fillText' && op.args[0] === ' / 15'));

  const skill = makeFakeCtx();
  drawSkillFigure(skill.ctx, 0, 80, 18, '5', { kind: 'missing', resolvedName: 'Attack' }, THEME);
  assert.ok(skill.ops.some(op => op.op === 'fillText' && op.args[0] === '5'));

  const strip = makeFakeCtx();
  const res = drawIconStrip(strip.ctx, 0, 90, 18, [
    { state: { kind: 'missing', resolvedName: 'A' }, name: 'A' },
    { state: { kind: 'missing', resolvedName: 'B' }, name: 'B' },
    { state: { kind: 'missing', resolvedName: 'C' }, name: 'C' },
    { state: { kind: 'missing', resolvedName: 'D' }, name: 'D' },
  ], THEME, { kind: 'traits' });
  assert.equal(res.drawnCount + res.overflowCount, 4);
  assert.ok(res.overflowCount > 0);
  assert.ok(!strip.ops.some(op => op.op === 'fillText' && String(op.args[0]).startsWith('+')), 'a strip that does not fit draws what fits, never a +N mark');

  const act = makeFakeCtx();
  const restBox = drawActionButton(act.ctx, 100, 18, 'View', false, THEME);
  const hoverBox = drawActionButton(act.ctx, 100, 18, 'View', true, THEME);
  assert.ok(hoverBox.width > restBox.width);
  assert.ok(act.ops.some(op => op.op === 'fillText' && op.args[0] === 'View'));

  const stock = makeFakeCtx();
  drawStockpileSumCell(stock.ctx, 0, 200, 0, 48, -120, 400, '-120', THEME);
  assert.ok(stock.ops.some(op => op.op === 'fillText' && op.args[0] === '-120'));

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

  assert.equal(contrastInkForFill('#f4f6f9', THEME), 'rgba(17, 17, 17, 0.88)');
  assert.equal(contrastInkForFill('#888888', THEME), 'rgba(17, 17, 17, 0.88)');
  assert.equal(contrastInkForFill('#1e4fa3', THEME), 'rgba(255, 255, 255, 0.96)');
  assert.equal(contrastInkForFill('#16181d', THEME), 'rgba(255, 255, 255, 0.96)');
  assert.equal(contrastInkForFill('not-a-colour', THEME), THEME.text);
});
