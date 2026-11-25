import { CHART_MEASURE_COLOURS } from '../../charts/palette.ts';
import {
  drawActionButton,
  drawArtSlot,
  drawBarBehindFigure,
  drawChevron,
  drawDateColumn,
  drawFlagSlot,
  drawIconStrip,
  drawMissingMark,
  drawNetBar,
  drawOutOfFigure,
  drawSkillFigure,
  drawSplitBar,
  drawStatusMark,
  drawTwoWayBar,
  drawTrackBar,
  drawWinRateCell,
  FLAG_SLOT_HEIGHT,
  FLAG_SLOT_WIDTH,
  resolveKitTokens,
  type ArtSlotState,
  type IconStripItem,
} from '../../draw/kit.ts';
import { resolveCachedArt, toArtSlotState } from '../art/adapters.ts';
import type { LaneArtCache } from '../art/laneCache.ts';
import { readyArt, type ArtResult } from '../art/types.ts';
import { formatValueState } from '../valueFormat.ts';
import { drawFormattedValue, drawHatchRect, MISSING_BOX_SIZE, PARTIAL_TAB_GAP, PARTIAL_TAB_HEIGHT, PARTIAL_TAB_WIDTH } from '../valueCanvas.ts';
import { DETAIL_LINE_HEIGHT, SHARE_FIGURE_SLOT, TREE_CHEVRON_LANE, SHARE_TRACK_GAP, SHARE_TRACK_HEIGHT, SHARE_TRACK_WIDTH, TWO_WAY_GAP, TWO_WAY_SLOT, type FamilyColumnLayout } from './layout.ts';
import type { CellPaintModel, FlatTableRow, TableCell } from './model.ts';
import {
  CELL_INSET,
  showsIcon,
  type TableBarsMode,
  type TableCanvasContext,
  type TableDensity,
  type TableTheme,
} from './settings.ts';

const CHILD_FLAG_GAP = 6;
const KIND_TILE_ART = { width: 20, height: 16 } as const;
function signedText(text: string, signed: number): string {
  if (signed > 0) return text.startsWith('+') ? text : `+${text}`;
  if (signed < 0) return text.replace(/^-/, '−');
  return text;
}

const TREE_LEVEL_LIMIT = 30;
const treeLevelCache = new WeakMap<readonly FlatTableRow[], number>();

export function treeLevelsOf(rows: readonly FlatTableRow[]): number {
  const held = treeLevelCache.get(rows);
  if (held !== undefined) return held;
  let levels = 0;
  for (let i = 0; i < rows.length; i++) {
    const item = rows[i]!;
    if (item.kind === 'data' && item.isTreeGroup) levels |= 1 << Math.min(item.level, TREE_LEVEL_LIMIT);
  }
  treeLevelCache.set(rows, levels);
  return levels;
}

export function treeTextStart(level: number, levels: number): number {
  let start = 0;
  for (let at = 1; at <= Math.min(level, TREE_LEVEL_LIMIT); at++) {
    if (levels & (1 << at)) start += TREE_CHEVRON_LANE;
  }
  return start;
}

export interface CellRenderContext {
  readonly ctx: TableCanvasContext;
  readonly theme: TableTheme;
  readonly density: TableDensity;
  readonly barsMode: TableBarsMode;
  readonly sortedColumn: string;
  readonly treeColumn: number;
  readonly treeLevels: number;
  readonly hasActionColumn?: boolean;
  readonly artCache?: LaneArtCache;
  readonly measure: (text: string, font: string) => number;
  readonly fit: (text: string, font: string, maxWidth: number) => string;
}

export interface CellDrawArgs {
  readonly labelLines?: readonly number[];
  readonly item: Extract<FlatTableRow, { kind: 'data' }>;
  readonly column: FamilyColumnLayout;
  readonly colIndex: number;
  readonly x: number;
  readonly y: number;
  readonly rowHeight: number;
  readonly paintModel: CellPaintModel;
  readonly formatted: ReturnType<typeof formatValueState>;
  readonly fraction: number | null;
  readonly fill: string;
  readonly rowHovered: boolean;
  readonly rowFocused: boolean;
  readonly lastChild?: boolean;
}

const fontOf = (theme: TableTheme, size: number, weight: number) => `${weight} ${size}px ${theme.font}`;

function slotOf(art: ArtResult | null | undefined, label: string | undefined, rc: CellRenderContext): ArtSlotState {
  return art ? toArtSlotState(resolveCachedArt(art, rc.artCache), label) : { kind: 'none' };
}

function artGeometry(shape: TableCell['artShape'], density: TableDensity, rowHeight: number): { readonly width: number; readonly height: number } {
  if (shape === 'portrait') {
    if (density === 'compact') return { width: 24, height: 32 };
    if (density === 'comfortable') return { width: 36, height: 48 };
    return { width: 30, height: 40 };
  }
  if (shape === 'equipment') return { width: 30, height: 24 };
  if (shape === 'counter') return { width: 30, height: 18 };
  if (shape === 'drawn') return { width: 28, height: 24 };
  if (shape === 'wide') {
    const h = Math.min(24, Math.max(18, rowHeight - 12));
    const w = Math.round(h * (46 / 28));
    return { width: w, height: h };
  }
  const s = Math.min(26, Math.max(18, rowHeight - 10));
  return { width: s, height: s };
}

function shouldDrawBehindBar(column: FamilyColumnLayout, rc: CellRenderContext): boolean {
  if (rc.barsMode === 'off') return false;
  if (column.barEligible === false) return false;
  if (column.barEligible === true || column.display === 'faint') {
    if (rc.barsMode === 'sorted') return column.key === rc.sortedColumn;
    return true;
  }
  return false;
}

function trackFillOf(column: FamilyColumnLayout, theme: TableTheme): string {
  const tokens = resolveKitTokens(theme);
  if (column.trackTone === 'red' || column.tone === 'red') return CHART_MEASURE_COLOURS.short;
  if (column.trackTone === 'grey' || column.tone === 'grey') return tokens.trackGrey;
  return theme.accent;
}

function drawInlineRuns(
  rc: CellRenderContext,
  text: string,
  x: number,
  middle: number,
  maxWidth: number,
  font: string,
  color: string,
): number {
  const { ctx, theme, measure, fit } = rc;
  if (maxWidth <= 0 || !text) return 0;
  if (text.indexOf('\uFFFC') < 0) {
    const fitted = fit(text, font, maxWidth);
    if (!fitted) return 0;
    ctx.save();
    ctx.font = font;
    ctx.fillStyle = color;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.fillText(fitted, x, middle);
    ctx.restore();
    return measure(fitted, font);
  }
  const parts = text.split('\uFFFC');
  const boxH = Math.max(MISSING_BOX_SIZE, 12);
  const boxPad = 4;
  let cursor = x;
  const limit = x + maxWidth;
  ctx.save();
  ctx.font = font;
  ctx.fillStyle = color;
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  for (let i = 0; i < parts.length; i++) {
    const part = parts[i]!;
    if (part) {
      const rem = limit - cursor;
      if (rem <= 0) break;
      const fitted = fit(part, font, rem);
      if (fitted) {
        ctx.fillText(fitted, cursor, middle);
        cursor += measure(fitted, font);
      }
      if (fitted !== part) break;
    }
    if (i + 1 < parts.length) {
      const lead = cursor > x ? boxPad : 0;
      const boxW = Math.min(48, Math.max(16, limit - cursor - lead));
      if (cursor + lead + 16 > limit) break;
      cursor += lead;
      drawHatchRect(ctx, cursor, Math.round(middle - boxH / 2), boxW, boxH, 2, theme);
      cursor += boxW + (parts[i + 1] ? boxPad : 0);
    }
  }
  ctx.restore();
  return cursor - x;
}

const GLYPHS = {
  helmet: { path: 'M2.6 10.8c0-3.4 2.4-6 5.4-6s5.4 2.6 5.4 6zM1.8 10.8h12.4M8 4.8v6', size: 14 },
  place: { path: 'M8 14s4.5-4.6 4.5-8a4.5 4.5 0 1 0-9 0C3.5 9.4 8 14 8 14Z M9.5 6a1.5 1.5 0 1 1-3 0a1.5 1.5 0 1 1 3 0', size: 12 },
  loss: { path: 'M4 4l8 8 M12 4l-8 8', size: 13 },
  transfer: { path: 'M2.5 5.5h11l-2.5-2.5 M13.5 10.5h-11l2.5 2.5', size: 13 },
} as const;
const GLYPH_GAP = 6;

const glyphs = new Map<keyof typeof GLYPHS, Path2D>();
function drawGlyph(rc: CellRenderContext, glyph: keyof typeof GLYPHS, x: number, mid: number, color: string): number {
  const { path: d, size } = GLYPHS[glyph];
  let path = glyphs.get(glyph);
  if (!path) { path = new Path2D(d); glyphs.set(glyph, path); }
  const ctx = rc.ctx;
  ctx.save();
  ctx.translate(x, mid - size / 2);
  ctx.scale(size / 16, size / 16);
  ctx.strokeStyle = color;
  ctx.lineWidth = 1.4;
  ctx.lineJoin = 'round';
  ctx.lineCap = 'round';
  ctx.stroke(path);
  ctx.restore();
  return size;
}

function glyphColor(rc: CellRenderContext, glyph: keyof typeof GLYPHS): string {
  const tokens = resolveKitTokens(rc.theme);
  if (glyph === 'helmet') return tokens.tileInk;
  return glyph === 'transfer' ? rc.theme.muted : tokens.quiet;
}

function drawKindTile(rc: CellRenderContext, tile: NonNullable<TableCell['artTile']>, centerX: number, middle: number): void {
  const { ctx, theme } = rc;
  const tokens = resolveKitTokens(theme);
  const width = tile === 'flat' ? 30 : 26;
  const height = tile === 'flat' ? 24 : 26;
  const x = Math.round(centerX - width / 2);
  const y = Math.round(middle - height / 2);
  ctx.save();
  ctx.beginPath();
  ctx.roundRect(x, y, width, height, 6);
  if (tile === 'flat') {
    ctx.fillStyle = tokens.tileFlat;
    ctx.fill();
  } else {
    const fill = ctx.createLinearGradient(x, y, x + width * 0.35, y + height);
    fill.addColorStop(0, tokens.tileTop);
    fill.addColorStop(1, tokens.tileBottom);
    ctx.fillStyle = fill;
    ctx.fill();
    ctx.strokeStyle = tokens.tileEdge;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.roundRect(x + 0.5, y + 0.5, width - 1, height - 1, 5.5);
    ctx.stroke();
  }
  ctx.restore();
}

export function drawApprovedCell(rc: CellRenderContext, args: CellDrawArgs): void {
  const { ctx, theme, density, measure, fit } = rc;
  const { item, column, colIndex, x, y, rowHeight, paintModel, formatted, fraction, rowHovered, rowFocused } = args;
  const cell = item.row.cells[column.key];
  const tokens = resolveKitTokens(theme);
  const leftBase = x + CELL_INSET + (column.leadInset ?? 0);
  const right = x + column.width - CELL_INSET - (column.trailInset ?? 0);
  const middle = y + rowHeight / 2;
  let left = leftBase;

  if (column.notRecordedReason || column.width === 0) return;
  const kind = column.cellKind;

  if (kind === 'action') {
    if (item.isTreeGroup && (cell?.blank || formatted.kind === 'notApplicable' || cell?.actionLabel === '')) {
      drawChevron(ctx, right - 10, middle, item.isOpen, theme);
      return;
    }
    if (cell?.blank || formatted.kind === 'notApplicable' || cell?.actionLabel === '') return;
    const label = cell?.actionLabel ?? 'View';
    drawActionButton(ctx, right, middle, label, rowHovered || rowFocused, theme, cell?.actionIcon);
    return;
  }

  if (cell?.blank) {
    if (kind === 'kind' && item.row.pageParent) {
      const branchX = Math.round(x + column.width / 2 - 8) + 0.5;
      ctx.save();
      ctx.strokeStyle = tokens.treeLine;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(branchX, y);
      ctx.lineTo(branchX, Math.round(middle) + 0.5);
      ctx.lineTo(branchX + 7, Math.round(middle) + 0.5);
      if (!args.lastChild) {
        ctx.moveTo(branchX, y + rowHeight - 10);
        ctx.lineTo(branchX, y + rowHeight);
      }
      ctx.stroke();
      ctx.restore();
    }
    return;
  }

  if (colIndex === rc.treeColumn) {
    if (rc.hasActionColumn || (item.row.pageGroup && kind === 'date')) {
      if (item.level > 1 && !item.row.pageParent) left += (item.level - 1) * TREE_CHEVRON_LANE;
    } else {
      left += treeTextStart(item.level, rc.treeLevels);
      if (item.isTreeGroup) drawChevron(ctx, left - TREE_CHEVRON_LANE - 2, middle, item.isOpen, theme);
      else if (item.level > 1 && cell?.iconSlot && !showsIcon(column.display)) {
        const state = slotOf(paintModel.art, paintModel.country ?? undefined, rc);
        drawFlagSlot(ctx, Math.max(x, left - FLAG_SLOT_WIDTH - CHILD_FLAG_GAP), Math.round(middle - FLAG_SLOT_HEIGHT / 2), state, theme);
      }
    }
  }

  if (kind === 'kind') {
    const centerX = x + (column.leadInset ?? 0) + (column.width - (column.leadInset ?? 0) - (column.trailInset ?? 0)) / 2;
    if (cell?.artTile) drawKindTile(rc, cell.artTile, centerX, middle);
    if (cell?.glyph) {
      drawGlyph(rc, cell.glyph, Math.round(centerX - GLYPHS[cell.glyph].size / 2), middle, glyphColor(rc, cell.glyph));
      return;
    }
    if (!cell || (!cell.art && !cell.iconSlot)) return;
    const geom = cell.artTile ? KIND_TILE_ART : artGeometry(cell.artShape ?? 'square', density, rowHeight);
    const state = slotOf(paintModel.art, cell.artName ?? item.name, rc);
    drawArtSlot(ctx, Math.round(centerX - geom.width / 2), Math.round(middle - geom.height / 2), geom.width, geom.height, state, theme,
      { radius: 2, trim: cell.artShape === 'drawn' });
    return;
  }
  if (cell?.glyph) {
    left += drawGlyph(rc, cell.glyph, left, middle, glyphColor(rc, cell.glyph)) + GLYPH_GAP;
  }

  if (kind === 'twoWay') {
    if (formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable') {
      drawFormattedValue(ctx, formatted, { left, right, middle, endAligned: true, muted: item.row.muted }, theme, measure, fit);
      return;
    }
    const signed = paintModel.value.kind === 'known' || paintModel.value.kind === 'partial' ? Number(paintModel.value.value) : 0;
    if (right - left >= TWO_WAY_SLOT + TWO_WAY_GAP) {
      const grey = tokens.twoWayNeutral;
      const share = fraction === null ? 0 : (signed < 0 ? -fraction : fraction);
      drawTwoWayBar(ctx, left + TWO_WAY_SLOT / 2, middle, TWO_WAY_SLOT / 2, 12, share, grey, grey, theme);
    }
    const font = fontOf(theme, 13, 500);
    ctx.save();
    ctx.font = font;
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    ctx.fillStyle = formatted.kind === 'zero' || item.row.muted ? theme.muted : theme.text;
    ctx.fillText(fit(signedText(formatted.text, signed), font, Math.max(0, right - left)), right, middle);
    ctx.restore();
    return;
  }

  if (kind === 'shaded') {
    if (formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable') {
      drawFormattedValue(ctx, formatted, { left, right, middle, endAligned: true, muted: item.row.muted }, theme, measure, fit);
      return;
    }
    const signed = paintModel.value.kind === 'known' || paintModel.value.kind === 'partial' ? Number(paintModel.value.value) : 0;
    if (signed !== 0 && fraction !== null) {
      ctx.save();
      ctx.fillStyle = signed > 0 ? theme.accent : tokens.shortage;
      ctx.globalAlpha = 0.18 + 0.5 * Math.max(0, Math.min(1, fraction));
      ctx.beginPath();
      ctx.roundRect(left, middle - Math.min(rowHeight - 4, 18) / 2, Math.max(0, right - left), Math.min(rowHeight - 4, 18), 3);
      ctx.fill();
      ctx.restore();
    }
    if (formatted.kind === 'zero') return;
    const font = fontOf(theme, 13, 500);
    ctx.save();
    ctx.font = font;
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    ctx.fillStyle = item.row.muted ? theme.muted : theme.text;
    ctx.fillText(fit(signedText(formatted.text, signed), font, Math.max(0, right - left - 6)), right - 4, middle);
    ctx.restore();
    return;
  }

  if (column.headerGroup?.kind === 'pair' && cell?.pair) {
    if (column.key === rc.sortedColumn) {
      ctx.save();
      ctx.fillStyle = theme.text;
      ctx.globalAlpha = 0.06;
      ctx.fillRect(x, y + 1, column.width, rowHeight - 2);
      ctx.restore();
    }
    const midpoint = (left + right) / 2;
    const bar = cell.splitBar;
    for (const [i, value] of cell.pair.entries()) {
      drawFormattedValue(ctx, formatValueState(value, paintModel.format), {
        left: i === 0 ? left : midpoint, right: i === 0 ? midpoint - CELL_INSET : right,
        middle: bar ? middle - 3 : middle, endAligned: true,
      }, theme, measure, fit);
    }
    if (bar) drawSplitBar(ctx, left, y + rowHeight - 6, right - left, 3, bar, theme);
    return;
  }

  if (kind === 'stockpileNet') {
    const sp = cell?.stockpile;
    if (!sp || sp.ai) return;
    drawNetBar(ctx, left, Math.max(0, right - left), y, rowHeight, sp.share, theme);
    return;
  }

  if (kind === 'stockpileSum' && cell?.stockpile) {
    const sp = cell.stockpile;
    const figureFont = fontOf(theme, 13.5, 500);
    const balanceFont = fontOf(theme, 13.5, 600);
    const operatorFont = fontOf(theme, 13.5, 400);
    const slot = Math.floor((right - left) / 3);
    ctx.save();
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    const figure = (text: string | null, slotRight: number, font: string, color: string, drawMissing: boolean) => {
      if (text === null) {
        if (drawMissing) drawMissingMark(ctx, slotRight - slot + 8, middle, slot - 8, theme, { align: 'right' });
        return;
      }
      ctx.font = font;
      ctx.fillStyle = color;
      ctx.fillText(fit(text, font, slot - 18), slotRight, middle);
    };
    const operator = (glyph: string, at: number) => {
      ctx.font = operatorFont;
      ctx.fillStyle = theme.muted;
      ctx.textAlign = 'center';
      ctx.fillText(glyph, at, middle);
      ctx.textAlign = 'right';
    };
    const isZero = (text: string | null) => text === '0' || text === '0.0';
    figure(sp.produced, left + slot - 14, figureFont, isZero(sp.produced) ? theme.muted : theme.text, true);
    if (!sp.ai) {
      operator('−', left + slot - 4);
      figure(sp.need, left + slot * 2 - 14, figureFont, isZero(sp.need) ? theme.muted : theme.text, true);
      operator('=', left + slot * 2 - 4);
      figure(sp.balance, right, balanceFont, sp.short ? tokens.shortage : (isZero(sp.balance) ? theme.muted : theme.text), true);
    }
    ctx.restore();
    return;
  }

  if (kind === 'stockpileBalance' && cell?.stockpile) {
    const sp = cell.stockpile;
    const top = middle - 9;
    const bottom = middle + 10;
    const lead = fontOf(theme, 13.5, 600);
    const word = fontOf(theme, 13.5, 500);
    const quiet = fontOf(theme, 12.5, 500);
    const runs = (parts: readonly (readonly [string, string, string])[], at: number) => {
      let cursor = right;
      ctx.textAlign = 'right';
      ctx.textBaseline = 'middle';
      for (let i = parts.length - 1; i >= 0; i--) {
        const [text, font, color] = parts[i]!;
        ctx.font = font;
        ctx.fillStyle = color;
        ctx.fillText(text, cursor, at);
        cursor -= measure(text, font);
        if (cursor < left) break;
      }
    };
    ctx.save();
    const secondary = tokens.dark ? '#d4d4d4' : theme.text;
    if (sp.ai) {
      runs(sp.produced === null ? [['Production not recorded', word, theme.muted]] : [[sp.produced, lead, theme.text], [' produced a day', word, theme.muted]], top);
      const reason = 'Need not recorded';
      ctx.font = quiet;
      const w = measure(reason, quiet);
      runs([[reason, quiet, theme.muted]], bottom);
      drawHatchRect(ctx, Math.round(right - w - MISSING_BOX_SIZE - 5), Math.round(bottom - MISSING_BOX_SIZE / 2), MISSING_BOX_SIZE, MISSING_BOX_SIZE, 2, theme);
    } else {
      runs(sp.balance === null ? [['Balance not recorded', word, theme.muted]] : [[sp.balance, lead, sp.short ? tokens.shortage : theme.text], [' a day', word, theme.text]], top);
      runs([
        [sp.produced ?? 'not recorded', quiet, secondary], [' produced · ', quiet, theme.muted],
        [sp.need ?? 'not recorded', quiet, secondary], [' needed', quiet, theme.muted],
      ], bottom);
    }
    ctx.restore();
    return;
  }

  if (kind === 'winRate') {
    const won = paintModel.value.kind === 'known' || paintModel.value.kind === 'zero' ? Number(paintModel.value.value) : null;
    const sec = cell?.secondaryValue;
    const lost = sec && (sec.kind === 'known' || sec.kind === 'zero') ? Number(sec.value) : null;
    if (formatted.kind === 'notRecorded') {
      drawMissingMark(ctx, left, middle, Math.max(0, right - left), theme, { align: 'right' });
      return;
    }
    drawWinRateCell(ctx, left, right, middle, won, lost, theme);
    return;
  }

  if (kind === 'skill') {
    if (formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable') {
      drawFormattedValue(ctx, formatted, { left, right, middle, endAligned: true, muted: item.row.muted }, theme, measure, fit);
      return;
    }
    const iconState = slotOf(paintModel.art ?? (column.skillIcon ? readyArt(column.skillIcon) : null), column.label, rc);
    drawSkillFigure(ctx, left, right, middle, formatted.text, iconState, theme, {
      badge: column.key === 'skill' || column.key === 'level',
      muted: item.row.muted,
    });
    return;
  }

  if (kind === 'icons') {
    if (formatted.kind === 'notRecorded') {
      drawMissingMark(ctx, left, middle, Math.max(0, right - left), theme, { align: 'left' });
      return;
    }
    const list = cell?.icons ?? [];
    if (!list.length) {
      ctx.save();
      ctx.font = fontOf(theme, 13, 400);
      ctx.fillStyle = theme.muted;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText('—', left, middle);
      ctx.restore();
      return;
    }
    const stripItems: IconStripItem[] = list.map(entry => ({
      state: slotOf(entry.art, entry.name, rc),
      name: entry.name,
      count: entry.count,
    }));
    drawIconStrip(ctx, left, right, middle, stripItems, theme, { kind: cell?.iconStripKind ?? 'traits' });
    return;
  }

  if (kind === 'date') {
    if (item.showDate === false) return;
    if (formatted.kind === 'notApplicable') {
      drawFormattedValue(ctx, formatted, { left, right, middle, endAligned: !!column.numeric, muted: item.row.muted }, theme, measure, fit);
      return;
    }
    const hourVal = paintModel.value.kind === 'known' || paintModel.value.kind === 'zero' || paintModel.value.kind === 'partial'
      ? Number(paintModel.value.value)
      : null;
    drawDateColumn(ctx, left, y, rowHeight, hourVal, theme, { width: Math.max(0, right - left), align: column.numeric ? 'right' : 'left' });
    return;
  }

  if (kind === 'status' || (cell?.statusKind && formatted.kind === 'known')) {
    if (formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable') {
      drawFormattedValue(ctx, formatted, { left, right, middle, endAligned: false, muted: item.row.muted }, theme, measure, fit);
      return;
    }
    if (cell?.statusKind) {
      drawStatusMark(ctx, left, middle, cell.statusKind, theme, {
        label: formatted.kind === 'known' ? formatted.text : undefined,
        maxWidth: Math.max(0, right - left),
      });
      return;
    }
  }

  if (kind === 'share') {
    drawShareCell(rc, item, column, left, right, middle, formatted, fraction, paintModel.countryColor);
    return;
  }

  if (kind === 'track' || column.display === 'track' || column.display === 'meterAfter' || column.display === 'meterBefore') {
    if (formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable') {
      drawFormattedValue(ctx, formatted, { left, right, middle, endAligned: true, muted: item.row.muted }, theme, measure, fit);
      return;
    }
    const valFont = fontOf(theme, 13.5, 500);
    const suffixFont = fontOf(theme, 12, 400);
    const suffix = cell?.unitSuffix ? ` ${cell.unitSuffix}` : '';
    const suffixW = suffix ? measure(suffix, suffixFont) : 0;
    const textW = measure(formatted.text, valFont);
    const numSlot = Math.max(42, Math.ceil(textW + suffixW));
    const trackW = Math.min(56, Math.max(18, right - left - numSlot - 10));
    const trackX = right - numSlot - 10 - trackW;
    if (trackW >= 18 && fraction !== null) {
      const barColor = paintModel.bar.alert ? CHART_MEASURE_COLOURS.short : trackFillOf(column, theme);
      drawTrackBar(ctx, trackX, middle, trackW, 6, fraction, barColor, theme);
    }
    let numRight = right;
    if (suffix && suffixW + 12 <= Math.max(0, right - left)) {
      ctx.save();
      ctx.font = suffixFont;
      ctx.fillStyle = theme.muted;
      ctx.textAlign = 'right';
      ctx.textBaseline = 'middle';
      ctx.fillText(suffix, right, middle);
      ctx.restore();
      numRight = right - suffixW;
    }
    ctx.save();
    ctx.font = valFont;
    ctx.fillStyle = item.row.muted || formatted.kind === 'zero' ? theme.muted : theme.text;
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    ctx.fillText(fit(formatted.text, valFont, Math.max(0, numRight - left)), numRight, middle);
    ctx.restore();
    return;
  }

  if (column.inlineFlags?.length && !cell?.inlinePrefix) {
    const marks = column.inlineFlags.flatMap(key => {
      const tail = item.row.cells[key];
      return tail && !tail.blank ? [tail] : [];
    });
    if (marks.length) {
      const gap = 6;
      const reserved = marks.reduce((sum, mark) => sum + (mark.glyph ? GLYPHS[mark.glyph].size + gap : 0)
        + (mark.flags?.length ?? 0) * (FLAG_SLOT_WIDTH + gap) + (!mark.flags?.length && mark.value.kind === 'notRecorded' ? 16 + gap : 0), 0);
      const room = Math.max(0, right - left - reserved);
      const font = fontOf(theme, density === 'compact' ? 13 : 13.5, 600);
      const missing = formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable';
      let at = left + (missing ? drawMissingMark(ctx, left, middle, room, theme, { figure: cell?.missingFigure }).width
        : drawInlineRuns(rc, formatted.text, left, middle, room, font, theme.text)) + gap;
      for (const mark of marks) {
        if (mark.glyph) at += drawGlyph(rc, mark.glyph, at, middle, glyphColor(rc, mark.glyph)) + gap;
        for (const flag of mark.flags ?? []) {
          drawFlagSlot(ctx, at, Math.round(middle - FLAG_SLOT_HEIGHT / 2), slotOf(flag.art, flag.tag, rc), theme, { dimmed: flag.dimmed });
          at += FLAG_SLOT_WIDTH + gap;
        }
        if (!mark.flags?.length && mark.value.kind === 'notRecorded') {
          drawMissingMark(ctx, at, middle, 16, theme);
          at += 16 + gap;
        }
      }
      return;
    }
  }

  if (cell?.quantities?.length) {
    for (const { figure, unit } of cell.quantities) {
      if (left >= right) break;
      left += drawInlineRuns(rc, figure, left, middle, right - left, fontOf(theme, 13, 600), theme.text) + 4;
      left += drawInlineRuns(rc, unit, left, middle, right - left, fontOf(theme, 13, 400), theme.muted) + 12;
    }
    return;
  }

  if (cell?.inlinePrefix && !column.numeric) {
    const font = fontOf(theme, density === 'compact' ? 13 : 13.5, 600);
    const room = Math.max(0, right - left);
    if (cell.flags?.length || measure(cell.inlinePrefix, font) + 10 < room) {
      left += drawInlineRuns(rc, cell.inlinePrefix, left, middle, room, font, item.row.muted ? theme.muted : theme.text) + 8;
      if (cell.flags?.length && right - left < FLAG_SLOT_WIDTH) return;
    }
  }

  if (cell?.flags && cell.flags.length > 0) {
    let fx = left;
    const fy = Math.round(middle - FLAG_SLOT_HEIGHT / 2);
    if (args.labelLines?.length) {
      const font = fontOf(theme, theme.fontSize.body, 400);
      for (const [line, start] of args.labelLines.entries()) {
        fx = left;
        const mid = y + (line + 0.5) * DETAIL_LINE_HEIGHT;
        const end = args.labelLines[line + 1] ?? cell.flags.length;
        for (let i = start; i < end; i++) {
          const flag = cell.flags[i]!;
          drawFlagSlot(ctx, fx, Math.round(mid - FLAG_SLOT_HEIGHT / 2), slotOf(flag.art, flag.tag, rc), theme, { dimmed: flag.dimmed });
          fx += FLAG_SLOT_WIDTH + 5;
          fx += drawInlineRuns(rc, flag.label ?? flag.tag, fx, mid, right - fx, font, tokens.childText) + 6;
        }
      }
      return;
    }
    for (const fl of cell.flags) {
      if (column.narrow === 'flag' && fx + FLAG_SLOT_WIDTH > right) break;
      const state = slotOf(fl.art, fl.tag, rc);
      drawFlagSlot(ctx, fx, fy, state, theme, { dimmed: fl.dimmed });
      fx += FLAG_SLOT_WIDTH + 6;
    }
    if (column.display === 'flag' || column.inlineFlags) return;
    left = fx + 2;
  } else if (cell?.artShape && (cell.art || cell.iconSlot)) {
    const geom = artGeometry(cell.artShape, density, rowHeight);
    const state = slotOf(paintModel.art, cell.artName ?? item.name, rc);
    drawArtSlot(ctx, left, Math.round(middle - geom.height / 2), geom.width, geom.height, state, theme, { radius: 3 });
    left += geom.width + 10;
  } else if (cell?.iconSlot && showsIcon(column.display)) {
    const fy = Math.round(middle - FLAG_SLOT_HEIGHT / 2);
    const isFlag = column.display === 'flag' || column.display === 'flagName';
    if (isFlag) {
      const state = slotOf(paintModel.art, paintModel.country ?? undefined, rc);
      drawFlagSlot(ctx, left, fy, state, theme);
      if (column.display === 'flag') return;
      left += FLAG_SLOT_WIDTH + 8;
    } else {
      const geom = artGeometry('square', density, rowHeight);
      const state = slotOf(paintModel.art, cell.artName ?? item.name, rc);
      drawArtSlot(ctx, left, Math.round(middle - geom.height / 2), geom.width, geom.height, state, theme, { radius: 3 });
      left += geom.width + 8;
    }
  }

  if (formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable') {
    if (kind === 'item' && formatted.kind === 'notRecorded' && !cell?.missingFigure) {
      const hatchW = Math.min(148, Math.max(0, right - left - 4));
      drawHatchRect(ctx, left, Math.round(middle - 6), hatchW, 12, 3, theme);
      return;
    }
    drawFormattedValue(ctx, formatted, { left, right, middle, endAligned: !!column.numeric, muted: item.row.muted }, theme, measure, fit);
    return;
  }

  if (kind === 'outOf') {
    const isSorted = column.key === rc.sortedColumn;
    if (shouldDrawBehindBar(column, rc) && fraction !== null && fraction > 0 && formatted.kind !== 'zero') {
      const barFill = paintModel.countryColor ?? tokens.barNeutral;
      drawBarBehindFigure(ctx, right, middle, fraction, barFill, { density, sorted: isSorted, theme });
    }
    const maxVal = cell?.secondaryValue;
    const maxFormatted = maxVal ? formatValueState(maxVal, paintModel.format) : null;
    const maxText = maxFormatted && (maxFormatted.kind === 'known' || maxFormatted.kind === 'zero' || maxFormatted.kind === 'partial') ? maxFormatted.text : '—';
    drawOutOfFigure(ctx, formatted.text, maxText, left, right, middle, theme, { muted: item.row.muted });
    return;
  }

  if (column.numeric) {
    const isSorted = column.key === rc.sortedColumn;
    const isZero = formatted.kind === 'zero';
    if (!isZero && shouldDrawBehindBar(column, rc) && fraction !== null && fraction > 0) {
      const barFill = paintModel.countryColor ?? tokens.barNeutral;
      drawBarBehindFigure(ctx, right, middle, fraction, barFill, { density, sorted: isSorted, theme });
    }
    const font = fontOf(theme, 13, 500);
    const hasSplit = Boolean(cell?.splitBar && cell.splitBar.length > 0);
    const textMid = hasSplit ? middle - 3 : middle;
    const avail = Math.max(0, right - left);
    const figureColor = cell?.shortage
      ? tokens.shortage
      : (isZero || item.row.muted)
        ? theme.muted
        : theme.text;
    let numRight = right;
    if (cell?.unitSuffix) {
      const suffixFont = fontOf(theme, 11.5, 400);
      const suffix = ` ${cell.unitSuffix}`;
      const suffixW = measure(suffix, suffixFont);
      if (suffixW + 12 <= avail) {
        ctx.save();
        ctx.font = suffixFont;
        ctx.textAlign = 'right';
        ctx.textBaseline = 'middle';
        ctx.fillStyle = theme.muted;
        ctx.fillText(suffix, right, textMid);
        ctx.restore();
        numRight = right - suffixW;
      }
    }
    const numAvail = Math.max(0, numRight - left);
    const shownNum = fit(formatted.text, font, numAvail);
    ctx.save();
    ctx.font = font;
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    ctx.fillStyle = figureColor;
    ctx.fillText(shownNum, numRight, textMid);
    ctx.restore();
    if (cell?.unitArt) {
      const iconSize = 14;
      const numW = measure(shownNum, font);
      const iconX = Math.round(numRight - numW - iconSize - 5);
      if (iconX >= left) {
        const state = slotOf(cell.unitArt, cell.unitSuffix ?? column.label, rc);
        drawArtSlot(ctx, iconX, Math.round(textMid - iconSize / 2), iconSize, iconSize, state, theme, { radius: 2 });
      }
    }
    if (hasSplit && cell?.splitBar) {
      const barW = Math.min(56, avail);
      if (barW >= 12) {
        drawSplitBar(ctx, right - barW, y + rowHeight - 7, barW, 3, cell.splitBar, theme);
      }
    }
    return;
  }

  const isPrimary = colIndex === rc.treeColumn || kind === 'item' || column.display === 'flagName';
  const isChild = item.level > 1;
  const subtext = density !== 'compact' ? cell?.subtext : undefined;
  const avail = Math.max(0, right - left);
  if (subtext && rowHeight >= 36) {
    const titleFont = fontOf(theme, 13.5, isChild ? 500 : 600);
    const subFont = fontOf(theme, 12, 400);
    const titleColor = item.row.muted ? theme.muted : (isChild ? tokens.childText : theme.text);
    drawInlineRuns(rc, formatted.text, left, middle - 8, avail, titleFont, titleColor);
    drawInlineRuns(rc, subtext, left, middle + 9, avail, subFont, theme.muted);
    return;
  }

  const textFont = fontOf(theme, density === 'compact' ? 13 : 13.5, column.textTone ? 400 : isChild ? 500 : (isPrimary ? 600 : 400));
  const textColor = item.row.muted || column.textTone === 'muted' || cell?.tone === 'muted'
    ? theme.muted
    : isChild || column.textTone === 'secondary'
      ? tokens.childText
      : theme.text;
  const drawnW = drawInlineRuns(rc, formatted.text, left, middle, avail, textFont, textColor);
  if (cell?.inlineSubtext && !column.inlineFlags) {
    const gap = 8;
    const rem = avail - drawnW - gap;
    if (rem >= 20) {
      const inlineFont = fontOf(theme, 12.5, 400);
      drawInlineRuns(rc, cell.inlineSubtext, left + drawnW + gap, middle, rem, inlineFont, theme.muted);
    }
  }
}


function drawShareCell(
  rc: CellRenderContext,
  item: Extract<FlatTableRow, { kind: 'data' }>,
  column: FamilyColumnLayout,
  left: number,
  right: number,
  middle: number,
  formatted: ReturnType<typeof formatValueState>,
  fraction: number | null,
  countryColor: string | null,
): void {
  const { ctx, theme, measure, fit } = rc;
  const muted = item.row.muted === true;
  if (formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable') {
    drawFormattedValue(ctx, formatted, { left, right, middle, endAligned: true, muted }, theme, measure, fit);
    return;
  }
  const font = fontOf(theme, 12, item.isTotal ? 600 : 500);
  const partial = formatted.kind === 'partial';
  const tab = partial ? PARTIAL_TAB_GAP + PARTIAL_TAB_WIDTH : 0;
  const figureRight = right - tab;
  if (rc.sortedColumn === column.key && !item.isTotal && fraction !== null) {
    const trackX = figureRight - SHARE_FIGURE_SLOT - SHARE_TRACK_GAP - SHARE_TRACK_WIDTH;
    if (trackX >= left) drawTrackBar(ctx, trackX, middle, SHARE_TRACK_WIDTH, SHARE_TRACK_HEIGHT, fraction, countryColor ?? theme.grey, theme);
  }
  ctx.save();
  ctx.font = font;
  ctx.fillStyle = muted || formatted.kind === 'zero' ? theme.muted : theme.text;
  ctx.textAlign = 'right';
  ctx.textBaseline = 'middle';
  const text = fit(formatted.text, font, Math.max(0, figureRight - left));
  ctx.fillText(text, figureRight, middle);
  ctx.restore();
  if (partial) drawHatchRect(ctx, right - PARTIAL_TAB_WIDTH, Math.round(middle - PARTIAL_TAB_HEIGHT / 2), PARTIAL_TAB_WIDTH, PARTIAL_TAB_HEIGHT, 1.5, theme);
}
