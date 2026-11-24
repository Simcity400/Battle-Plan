import { LABEL_WEIGHT } from '../../charts/draw.ts';
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
  type IconStripItem,
} from '../../draw/kit.ts';
import { resolveCachedArt, toArtSlotState, toFlagSlotState } from '../art/adapters.ts';
import type { LaneArtCache } from '../art/laneCache.ts';
import { PENDING_ART, readyArt, unavailableArt } from '../art/types.ts';
import { formatDateParts, formatValueState } from '../valueFormat.ts';
import { drawFormattedValue, drawHatchRect, MISSING_BOX_SIZE, PARTIAL_TAB_GAP, PARTIAL_TAB_HEIGHT, PARTIAL_TAB_WIDTH } from '../valueCanvas.ts';
import { SHARE_FIGURE_SLOT, TREE_CHEVRON_LANE, SHARE_TRACK_GAP, SHARE_TRACK_HEIGHT, SHARE_TRACK_WIDTH, TWO_WAY_GAP, TWO_WAY_SLOT, type FamilyColumnLayout } from './layout.ts';
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
  readonly icons?: ReadonlyMap<string, ImageBitmap | null>;
  readonly artCache?: LaneArtCache;
  readonly measure: (text: string, font: string) => number;
  readonly fit: (text: string, font: string, maxWidth: number) => string;
}

export interface CellDrawArgs {
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
}

const fontOf = (theme: TableTheme, size: number, weight: number) => `${weight} ${size}px ${theme.font}`;

function cellArt(url: string | null | undefined, pending: boolean, rc: CellRenderContext) {
  const resolution = url ? readyArt(url) : pending ? PENDING_ART : unavailableArt('Artwork unavailable');
  return resolveCachedArt(resolution, { artCache: rc.artCache, icons: rc.icons });
}

function artGeometry(shape: TableCell['artShape'], density: TableDensity, rowHeight: number): { readonly width: number; readonly height: number } {
  if (shape === 'portrait') {
    if (density === 'compact') return { width: 24, height: 32 };
    if (density === 'comfortable') return { width: 36, height: 48 };
    return { width: 30, height: 40 };
  }
  if (shape === 'equipment') return { width: 30, height: 24 };
  if (shape === 'counter') return { width: 30, height: 18 };
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

const BOUNDED_DATE_WORDS: Readonly<Record<'by' | 'since' | 'after' | 'before', string>> = {
  by: 'by',
  since: 'since',
  after: 'after',
  before: 'before',
};

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

export function drawApprovedCell(rc: CellRenderContext, args: CellDrawArgs): void {
  const { ctx, theme, density, measure, fit } = rc;
  const { item, column, colIndex, x, y, rowHeight, paintModel, formatted, fraction, rowHovered, rowFocused } = args;
  const cell = item.row.cells[column.key];
  const tokens = resolveKitTokens(theme);
  const leftBase = x + CELL_INSET + (column.leadInset ?? 0);
  const right = x + column.width - CELL_INSET - (column.trailInset ?? 0);
  const middle = y + rowHeight / 2;
  let left = leftBase;

  if (column.notRecordedReason) return;
  const kind = column.cellKind;

  if (kind === 'action') {
    if (item.isTreeGroup && (cell?.blank || formatted.kind === 'notApplicable' || cell?.actionLabel === '')) {
      drawChevron(ctx, right - 10, middle, item.isOpen, theme);
      return;
    }
    if (cell?.blank || formatted.kind === 'notApplicable' || cell?.actionLabel === '') return;
    const label = cell?.actionLabel ?? 'View';
    drawActionButton(ctx, right, middle, label, rowHovered || rowFocused, theme);
    return;
  }

  if (cell?.blank) return;

  if (colIndex === rc.treeColumn) {
    if (rc.hasActionColumn) {
      if (item.level > 1) left += (item.level - 1) * TREE_CHEVRON_LANE;
    } else {
      left += treeTextStart(item.level, rc.treeLevels);
      if (item.isTreeGroup) drawChevron(ctx, left - TREE_CHEVRON_LANE - 2, middle, item.isOpen, theme);
      else if (item.level > 1 && cell?.iconSlot && !showsIcon(column.display)) {
        const state = toFlagSlotState(cellArt(paintModel.icon, paintModel.iconPending === true, rc), paintModel.country ?? undefined);
        drawFlagSlot(ctx, Math.max(x, left - FLAG_SLOT_WIDTH - CHILD_FLAG_GAP), Math.round(middle - FLAG_SLOT_HEIGHT / 2), state, theme);
      }
    }
  }

  if (kind === 'kind') {
    if (!cell || (!cell.icon && !paintModel.iconPending && !cell.iconSlot)) return;
    const geom = artGeometry(cell.artShape ?? 'square', density, rowHeight);
    const state = toArtSlotState(cellArt(paintModel.icon, paintModel.iconPending === true, rc), cell.artName ?? item.name);
    drawArtSlot(ctx, Math.round(x + (column.width - geom.width) / 2), Math.round(middle - geom.height / 2), geom.width, geom.height, state, theme, { radius: 2 });
    return;
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
    const figure = (text: string | null, slotRight: number, font: string, color: string) => {
      if (text === null) {
        if (!sp.ai) drawMissingMark(ctx, slotRight - slot + 8, middle, slot - 8, theme, { align: 'right' });
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
    figure(sp.produced, left + slot - 14, figureFont, theme.text);
    if (!sp.ai) {
      operator('−', left + slot - 4);
      figure(sp.need, left + slot * 2 - 14, figureFont, theme.text);
      operator('=', left + slot * 2 - 4);
      figure(sp.balance, right, balanceFont, sp.short ? tokens.shortage : theme.text);
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
    const iconUrl = cell?.icon ?? column.skillIcon ?? null;
    const iconState = iconUrl
      ? toArtSlotState(cellArt(iconUrl, paintModel.iconPending === true, rc), column.label)
      : { kind: 'none' as const };
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
      state: toArtSlotState(cellArt(entry.url, entry.pending === true, rc), entry.name),
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
    const width = Math.max(0, right - left);
    if (cell?.boundedDate && hourVal !== null) {
      const parts = formatDateParts(hourVal);
      if (parts) {
        const prefix = `${BOUNDED_DATE_WORDS[cell.boundedDate]} `;
        const dayFont = fontOf(theme, rowHeight >= 32 ? 12.5 : 11.5, 500);
        const yearFont = fontOf(theme, 10.5, 400);
        const prefixW = measure(prefix, dayFont);
        const dayY = rowHeight >= 32 ? middle - 6 : middle;
        ctx.save();
        ctx.textAlign = 'left';
        ctx.textBaseline = 'middle';
        ctx.font = dayFont;
        if (prefixW < width - 8) {
          ctx.fillStyle = theme.muted;
          ctx.fillText(prefix, left, dayY);
          ctx.fillStyle = theme.text;
          ctx.fillText(fit(parts.dayMonth, dayFont, Math.max(0, width - prefixW)), left + prefixW, dayY);
        } else {
          ctx.fillStyle = theme.text;
          ctx.fillText(fit(`${prefix}${parts.dayMonth}`, dayFont, width), left, dayY);
        }
        if (rowHeight >= 32) {
          ctx.font = yearFont;
          ctx.fillStyle = theme.muted;
          ctx.fillText(fit(parts.year, yearFont, width), left, middle + 7);
        }
        ctx.restore();
        return;
      }
    }
    drawDateColumn(ctx, left, y, rowHeight, hourVal, theme, { width, align: column.numeric ? 'right' : 'left' });
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

  if (cell?.flags && cell.flags.length > 0) {
    let fx = left;
    const fy = Math.round(middle - FLAG_SLOT_HEIGHT / 2);
    for (const fl of cell.flags) {
      const state = toFlagSlotState(cellArt(fl.icon, fl.pending === true, rc), fl.tag);
      drawFlagSlot(ctx, fx, fy, state, theme, { dimmed: fl.dimmed });
      fx += FLAG_SLOT_WIDTH + 6;
    }
    if (column.display === 'flag') return;
    left = fx + 2;
  } else if (cell?.artShape && (cell.icon || paintModel.iconPending || cell.iconSlot)) {
    const geom = artGeometry(cell.artShape, density, rowHeight);
    const state = toArtSlotState(cellArt(paintModel.icon, paintModel.iconPending === true, rc), cell.artName ?? item.name);
    drawArtSlot(ctx, left, Math.round(middle - geom.height / 2), geom.width, geom.height, state, theme, { radius: 3 });
    left += geom.width + 10;
  } else if (cell?.iconSlot && showsIcon(column.display)) {
    const fy = Math.round(middle - FLAG_SLOT_HEIGHT / 2);
    const isFlag = column.display === 'flag' || column.display === 'flagName';
    if (isFlag) {
      const state = toFlagSlotState(cellArt(paintModel.icon, paintModel.iconPending === true, rc), paintModel.country ?? undefined);
      drawFlagSlot(ctx, left, fy, state, theme);
      if (column.display === 'flag') return;
      left += FLAG_SLOT_WIDTH + 8;
    } else {
      const geom = artGeometry('square', density, rowHeight);
      const state = toArtSlotState(cellArt(paintModel.icon, paintModel.iconPending === true, rc), cell.artName ?? item.name);
      drawArtSlot(ctx, left, Math.round(middle - geom.height / 2), geom.width, geom.height, state, theme, { radius: 3 });
      left += geom.width + 8;
    }
  }

  if (paintModel.missingName) {
    const font = fontOf(theme, 13, LABEL_WEIGHT);
    ctx.save();
    ctx.font = font;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.fillText(fit('Name not recorded', font, Math.max(0, right - left)), left, middle);
    ctx.restore();
    return;
  }

  if (formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable') {
    let nrLeft = left;
    if (cell?.inlinePrefix && !column.numeric) {
      const prefixFont = fontOf(theme, density === 'compact' ? 13 : 13.5, 600);
      const prefixW = measure(cell.inlinePrefix, prefixFont);
      if (prefixW + 10 < Math.max(0, right - nrLeft)) {
        ctx.save();
        ctx.font = prefixFont;
        ctx.fillStyle = item.row.muted ? theme.muted : theme.text;
        ctx.textAlign = 'left';
        ctx.textBaseline = 'middle';
        ctx.fillText(cell.inlinePrefix, nrLeft, middle);
        ctx.restore();
        nrLeft += prefixW + 8;
      }
    }
    if (kind === 'item' && formatted.kind === 'notRecorded' && !cell?.missingFigure) {
      const hatchW = Math.min(148, Math.max(48, right - nrLeft - 8));
      drawHatchRect(ctx, nrLeft, Math.round(middle - 6), hatchW, 12, 3, theme);
      return;
    }
    drawFormattedValue(ctx, formatted, { left: nrLeft, right, middle, endAligned: !!column.numeric, muted: item.row.muted }, theme, measure, fit);
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
    if (cell?.unitIcon || cell?.unitIconPending) {
      const iconSize = 14;
      const numW = measure(shownNum, font);
      const iconX = Math.round(numRight - numW - iconSize - 5);
      if (iconX >= left) {
        const state = toArtSlotState(cellArt(cell.unitIcon, cell.unitIconPending === true, rc), cell.unitSuffix ?? column.label);
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
  let textLeft = left;
  if (cell?.inlinePrefix) {
    const prefixFont = fontOf(theme, density === 'compact' ? 13 : 13.5, 600);
    const prefixW = measure(cell.inlinePrefix, prefixFont);
    if (prefixW + 10 < Math.max(0, right - textLeft)) {
      ctx.save();
      ctx.font = prefixFont;
      ctx.fillStyle = item.row.muted ? theme.muted : theme.text;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText(cell.inlinePrefix, textLeft, middle);
      ctx.restore();
      textLeft += prefixW + 8;
    }
  }
  const avail = Math.max(0, right - textLeft);
  if (subtext && rowHeight >= 36) {
    const titleFont = fontOf(theme, 13.5, isChild ? 500 : 600);
    const subFont = fontOf(theme, 12, 400);
    const titleColor = item.row.muted ? theme.muted : (isChild ? tokens.childText : theme.text);
    drawInlineRuns(rc, formatted.text, textLeft, middle - 8, avail, titleFont, titleColor);
    drawInlineRuns(rc, subtext, textLeft, middle + 9, avail, subFont, theme.muted);
    return;
  }

  const textFont = fontOf(theme, density === 'compact' ? 13 : 13.5, isChild ? 500 : (isPrimary ? 600 : 400));
  const textColor = item.row.muted
    ? theme.muted
    : isChild
      ? tokens.childText
      : theme.text;
  const drawnW = drawInlineRuns(rc, formatted.text, textLeft, middle, avail, textFont, textColor);
  if (cell?.inlineSubtext) {
    const gap = 8;
    const rem = avail - drawnW - gap;
    if (rem >= 20) {
      const inlineFont = fontOf(theme, 12.5, 400);
      drawInlineRuns(rc, cell.inlineSubtext, textLeft + drawnW + gap, middle, rem, inlineFont, theme.muted);
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
