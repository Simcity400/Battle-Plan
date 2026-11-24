import assert from 'node:assert/strict';
import test from 'node:test';
import { engineHour } from '../dashboard/calendar.ts';
import type { FamilyTheme } from '../families/contract.ts';
import {
  BEHIND_BAR_HEIGHT,
  BEHIND_BAR_MAX_WIDTH,
  BEHIND_BAR_OFFSET,
  FLAG_LOSER_ALPHA,
  FLAG_SLOT_HEIGHT,
  FLAG_SLOT_WIDTH,
  artInitials,
  barCornerRadius,
  drawArtSlot,
  drawBarBehindFigure,
  drawDateColumn,
  drawDateTab,
  drawFlagSlot,
  drawGapBand,
  drawGradientLegend,
  drawMissingMark,
  drawNameSlot,
  drawPartialTab,
  drawRowHairline,
  drawRowHover,
  drawRowSelected,
  drawSeriesMissingStrip,
  drawSplitBar,
  drawStatusMark,
  drawTrackBar,
  drawTwoWayBar,
  resolveKitTokens,
  TIMELINE_BAR_ICON_MIN_WIDTH,
  computeTimelineTicks,
  drawBottomTimeAxis,
  drawChevron,
  drawSparkline,
  drawSplitWarBar,
  drawThresholdMeter,
  drawTimelineSpanBar,
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
  const hoverTab = drawDateTab(chromeCtx.ctx, '1 Sep 1939', 100, 120, THEME, 'hover', { left: 10, right: 200 });
  assert.ok(hoverTab.width > 0);
  const tokens = resolveKitTokens(THEME);
  assert.equal(tokens.shortage, '#ff6b6f');
  assert.equal(tokens.dateTabBg, THEME.raised);
  assert.equal(tokens.hoverTabBg, THEME.text);
});

test('timeline primitives handle notches, narrow icon/name threshold, split war diamonds, bottom axis and meters', () => {
  const bitmap = { width: 32, height: 32 } as unknown as CanvasImageSource;
  const narrow = makeFakeCtx();
  const narrowRes = drawTimelineSpanBar(narrow.ctx, {
    x: 10, y: 10, width: TIMELINE_BAR_ICON_MIN_WIDTH - 2, height: 18, fill: '#3987e5', theme: THEME,
    icon: { kind: 'ready', bitmap, width: 32, height: 32 }, label: 'Free Trade',
  });
  assert.equal(narrowRes.showedIcon, false);
  assert.equal(narrowRes.showedLabel, false);

  const wide = makeFakeCtx();
  const wideRes = drawTimelineSpanBar(wide.ctx, {
    x: 10, y: 10, width: 120, height: 18, fill: '#3987e5', theme: THEME,
    heldBefore: true, openAtPlayhead: true,
    icon: { kind: 'ready', bitmap, width: 32, height: 32 }, label: 'Free Trade',
  });
  assert.equal(wideRes.showedIcon, true);
  assert.equal(wideRes.showedLabel, true);
  assert.ok(wide.ops.some(op => op.op === 'drawImage'));
  assert.ok(wide.ops.some(op => op.op === 'fillText' && op.args[0] === 'Free Trade'));

  const shortWar = makeFakeCtx();
  const shortRes = drawSplitWarBar(shortWar.ctx, {
    x: 40, y: 8, width: 6, height: 14, topFill: '#3987e5', bottomFill: '#d95926', theme: THEME,
  });
  assert.equal(shortRes.diamond, true);

  const longWar = makeFakeCtx();
  const longRes = drawSplitWarBar(longWar.ctx, {
    x: 40, y: 8, width: 90, height: 14, topFill: '#3987e5', bottomFill: '#d95926', theme: THEME, openAtPlayhead: true,
  });
  assert.equal(longRes.diamond, false);
  assert.equal(longWar.ops.filter(op => op.op === 'roundRect').length, 2);

  const h1936 = engineHour({ year: 1936, month: 1, day: 1, hour: 1 })!;
  const h1940 = engineHour({ year: 1940, month: 6, day: 15, hour: 1 })!;
  const h1937 = engineHour({ year: 1937, month: 3, day: 1, hour: 1 })!;
  const yearTicks = computeTimelineTicks(h1936, h1940, 400);
  assert.ok(yearTicks.some(t => t.label === '1937'));
  const monthTicks = computeTimelineTicks(h1936, h1937, 400);
  assert.ok(monthTicks.some(t => t.label.startsWith('Jan')));

  const axis = makeFakeCtx();
  const axisRes = drawBottomTimeAxis(axis.ctx, {
    plotLeft: 180, plotRight: 580, plotTop: 0, axisTop: 200,
    startHour: h1936, endHour: h1940, playheadHour: h1940, theme: THEME,
  });
  assert.ok(axisRes.playheadX !== null);
  assert.ok(axis.ops.some(op => op.op === 'fillText' && op.args[0] === '15 Jun 1940'));

  const extra = makeFakeCtx();
  drawChevron(extra.ctx, 4, 16, true, THEME.muted);
  drawThresholdMeter(extra.ctx, 10, 16, 120, 7, 0.75, '#3987e5', THEME);
  drawSparkline(extra.ctx, 140, 6, 48, 20, [0.2, 0.4, null, 0.7, 0.8], '#3987e5', THEME);
  assert.ok(extra.ops.some(op => op.op === 'stroke'));
});
