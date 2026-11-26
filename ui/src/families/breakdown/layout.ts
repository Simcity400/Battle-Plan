import { contrastInkForFill } from '../../draw/kit.ts';
import { shadeColour, tintColour } from '../../shell/appearance/color.ts';
import type { FamilyTheme } from '../contract.ts';
import { formatShare } from '../valueFormat.ts';
import type { BreakdownViewMode } from './options.ts';
import {
  childTint,
  drawnChildren,
  COMPARISON_KEY,
  PARTIAL_KEY,
  partColour,
  resolveExpandedMap,
  resolveRingOpenKey,
  rowsBelow,
  type BreakdownComparison,
  type BreakdownPart,
  type BreakdownTotal,
  type ResolvedBreakdownLevel,
} from './model.ts';

export type MeasureTextFn = (text: string, font: string) => number;

export interface Rect {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

export type BreakdownDepth = 0 | 1 | 2;

export interface DrawnItem {
  readonly key: string;
  readonly parentKey: string | null;
  readonly depth: BreakdownDepth;
  readonly part: BreakdownPart;
  readonly open: -1 | 0 | 1;
  readonly shareText: string;
  readonly top?: number;
  readonly height?: number;
}

export interface TotalLineLayout {
  readonly rect: Rect;
  readonly narrow: boolean;
  readonly total: BreakdownTotal;
  readonly periodLabel?: string;
  readonly comparison?: BreakdownComparison;
  readonly comparisonRect?: Rect;
  readonly partialTabRect?: Rect;
}

export interface PartsSegment {
  readonly key: string;
  readonly name: string;
  readonly text: string;
  readonly x: number;
  readonly width: number;
  readonly color: string;
  readonly inlineLabel: string | null;
  readonly ink: string;
  readonly notRecorded: boolean;
}

export interface PartsRowLayout {
  readonly key: string;
  readonly parentKey: string | null;
  readonly depth: BreakdownDepth;
  readonly part: BreakdownPart;
  readonly y: number;
  readonly height: number;
  readonly swatchColor: string;
  readonly hasChildren: boolean;
  readonly expanded: boolean;
  readonly nameX: number;
  readonly nameWidth: number;
  readonly chevronRect: Rect | null;
  readonly barX: number;
  readonly barWidth: number;
  readonly barFillWidth: number;
  readonly segments: readonly PartsSegment[];
  readonly valueX: number;
  readonly valueWidth: number;
  readonly shareX: number;
  readonly shareWidth: number;
  readonly shareText: string;
  readonly isLastChild: boolean;
}

export interface PartsViewLayout {
  readonly view: 'parts';
  readonly width: number;
  readonly height: number;
  readonly narrow: boolean;
  readonly padX: number;
  readonly totalLine: TotalLineLayout;
  readonly bodyTop: number;
  readonly rows: readonly PartsRowLayout[];
  readonly contentHeight: number;
  readonly expanded: Readonly<Record<string, boolean>>;
  readonly items: readonly DrawnItem[];
}

export interface RingInnerSlice {
  readonly key: string;
  readonly part: BreakdownPart;
  readonly a0: number;
  readonly a1: number;
  readonly color: string;
  readonly hatched: boolean;
  readonly label: {
    readonly x: number;
    readonly y: number;
    readonly line1: string;
    readonly line2: string | null;
    readonly ink: string;
    readonly fontSize: number;
  } | null;
}

export interface RingOuterSlice {
  readonly key: string;
  readonly parentKey: string;
  readonly part: BreakdownPart;
  readonly a0: number;
  readonly a1: number;
  readonly color: string;
  readonly hatched: boolean;
  readonly arcLabel: {
    readonly text: string;
    readonly midAngle: number;
    readonly reversed: boolean;
    readonly ink: string;
  } | null;
}

export interface RingSideRow {
  readonly key: string;
  readonly parentKey: string | null;
  readonly depth: BreakdownDepth;
  readonly part: BreakdownPart;
  readonly y: number;
  readonly height: number;
  readonly swatchColor: string;
  readonly hasChildren: boolean;
  readonly expanded: boolean;
  readonly chevronRect: Rect | null;
  readonly shareText: string;
}

export interface RingViewLayout {
  readonly view: 'ring';
  readonly width: number;
  readonly height: number;
  readonly narrow: boolean;
  readonly padX: number;
  readonly topLine: {
    readonly rect: Rect;
    readonly text: string;
    readonly comparison?: BreakdownComparison;
    readonly comparisonRect?: Rect;
    readonly partialTabRect?: Rect;
  } | null;
  readonly cx: number;
  readonly cy: number;
  readonly r1i: number;
  readonly r1o: number;
  readonly r2i: number;
  readonly r2o: number;
  readonly innerSlices: readonly RingInnerSlice[];
  readonly outerSlices: readonly RingOuterSlice[];
  readonly center: {
    readonly verb: string | null;
    readonly value: string;
    readonly valueFontSize: number;
    readonly unitLine: string | null;
    readonly partialTabRect: Rect | null;
  };
  readonly sideList: {
    readonly x: number;
    readonly top: number;
    readonly width: number;
    readonly height: number;
    readonly rows: readonly RingSideRow[];
    readonly contentHeight: number;
  } | null;
  readonly openKey: string | null;
  readonly contentHeight: number;
  readonly expanded: Readonly<Record<string, boolean>>;
  readonly items: readonly DrawnItem[];
}

export interface MosaicTile {
  readonly key: string;
  readonly parentKey: string | null;
  readonly part: BreakdownPart;
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
  readonly color: string;
  readonly ink: string;
  readonly hatched: boolean;
  readonly isGroup: boolean;
  readonly headerHeight: number;
  readonly headerRight: string | null;
  readonly labelMode: 'full' | 'name' | 'none';
  readonly valueLine: string;
  readonly shareText: string;
  readonly children: readonly MosaicTile[];
}

export interface MosaicViewLayout {
  readonly view: 'mosaic';
  readonly width: number;
  readonly height: number;
  readonly narrow: boolean;
  readonly padX: number;
  readonly totalLine: TotalLineLayout;
  readonly area: Rect;
  readonly tiles: readonly MosaicTile[];
  readonly contentHeight: number;
  readonly expanded: Readonly<Record<string, boolean>>;
  readonly items: readonly DrawnItem[];
  readonly undrawn: string | null;
}

export type BreakdownLayout = PartsViewLayout | RingViewLayout | MosaicViewLayout;

export interface BreakdownLayoutInput {
  readonly view: BreakdownViewMode;
  readonly level: ResolvedBreakdownLevel;
  readonly periodLabel?: string;
  readonly comparison?: BreakdownComparison;
  readonly width: number;
  readonly height: number;
  readonly theme: FamilyTheme;
  readonly namesInRing: boolean;
  readonly userExpanded: Readonly<Record<string, boolean>>;
  readonly measure: MeasureTextFn;
}

const NARROW_BREAKPOINT = 420;
const TOTAL_LINE_HEIGHT = 34;
const PARTS_ROW_HEIGHT = 23;
const PARTS_CHILD_HEIGHT = 18;
const PARTS_ITEM_HEIGHT = 17;
const RING_SIDE_ROW_HEIGHT = 20;
const RING_TOP_LINE_HEIGHT = 28;
const MOSAIC_GROUP_HEADER = 20;
const SEGMENT_TINTS = [0, 0.2] as const;
const OUTER_TINTS = [0.12, 0.3] as const;
const TILE_TINTS = [0, 0.14, 0.26, 0.07] as const;

export function layoutBreakdown(input: BreakdownLayoutInput): BreakdownLayout {
  if (input.view === 'ring') return layoutRingView(input);
  if (input.view === 'mosaic') return layoutMosaicView(input);
  return layoutPartsView(input);
}

function shareOf(part: BreakdownPart, total: BreakdownTotal): string {
  return total.atLeast || part.notRecorded || part.share == null ? '' : formatShare(part.share);
}

function positive(part: BreakdownPart): boolean {
  return !part.notRecorded && part.value !== null && part.value > 0;
}

function subdivision(shown: readonly BreakdownPart[]): readonly number[] | null {
  if (!shown.length || !shown.every(child => child.parentShare != null)) return null;
  const shares = shown.map(child => child.parentShare!);
  return shares.reduce((sum, share) => sum + share, 0) <= 1 + 1e-9 ? shares : null;
}

function wholeOf(level: ResolvedBreakdownLevel, drawn: number): number {
  return level.total.known && level.total.value !== null && level.total.value > drawn ? level.total.value : drawn;
}

function undrawnParts(parts: readonly BreakdownPart[], drawn: ReadonlySet<string>, total: BreakdownTotal): DrawnItem[] {
  return parts.filter(part => !drawn.has(part.key)).map(part => ({ key: part.key, parentKey: null, depth: 0 as const, part, open: -1, shareText: shareOf(part, total) }));
}

function undrawnNote(parts: readonly BreakdownPart[], drawnCount: number, total: BreakdownTotal): string | null {
  if (drawnCount > 0 || !parts.length) return null;
  const reason = total.reason ?? parts.find(part => part.missingReason)?.missingReason;
  return reason ? `Nothing here can be drawn to scale: ${reason}` : 'Nothing here can be drawn to scale.';
}

function comparisonBox(
  comparison: BreakdownComparison | undefined,
  right: number,
  left: number,
  y: number,
  height: number,
  theme: FamilyTheme,
  measure: MeasureTextFn,
): Rect | undefined {
  if (!comparison) return undefined;
  const width = measure(comparison.label, `400 12px ${theme.font}`) + 6 + measure(comparison.value, `500 12px ${theme.font}`) + (comparison.partial ? 12 : 0);
  return right - width > left + 16 ? { x: right - width, y, width, height } : undefined;
}

function layoutTotalLine(
  x: number,
  y: number,
  width: number,
  narrow: boolean,
  total: BreakdownTotal,
  periodLabel: string | undefined,
  comparison: BreakdownComparison | undefined,
  theme: FamilyTheme,
  measure: MeasureTextFn,
): TotalLineLayout {
  let cursor = x;
  cursor += measure(total.text, `600 22px ${theme.font}`);
  if (total.unit) cursor += 4 + measure(total.unit, `400 12px ${theme.font}`);
  let partialTabRect: Rect | undefined;
  if (total.atLeast || total.partial) {
    partialTabRect = { x: cursor + 6, y: y + Math.round((TOTAL_LINE_HEIGHT - 13) / 2), width: 6, height: 13 };
    cursor = partialTabRect.x + partialTabRect.width;
  }
  const comparisonRect = narrow ? undefined : comparisonBox(comparison, x + width, cursor, y, TOTAL_LINE_HEIGHT, theme, measure);
  return {
    rect: { x, y, width, height: TOTAL_LINE_HEIGHT },
    narrow,
    total,
    periodLabel: narrow ? undefined : periodLabel,
    ...(comparisonRect && comparison ? { comparison, comparisonRect } : {}),
    partialTabRect,
  };
}

function layoutPartsView(input: BreakdownLayoutInput): PartsViewLayout {
  const { level, periodLabel, comparison, width, height, theme, userExpanded, measure } = input;
  const narrow = width < NARROW_BREAKPOINT;
  const padX = narrow ? 10 : 16;
  const padTop = narrow ? 6 : 8;
  const padBottom = 10;
  const innerW = Math.max(80, width - padX * 2);

  const totalLine = layoutTotalLine(padX, padTop, innerW, narrow, level.total, periodLabel, comparison, theme, measure);
  const bodyTop = padTop + TOTAL_LINE_HEIGHT + 4;

  const visibleRows = Math.max(1, Math.floor((height - bodyTop - padBottom) / PARTS_ROW_HEIGHT));
  const expanded = resolveExpandedMap(level.parts, visibleRows, narrow, userExpanded);

  const gap = narrow ? 8 : 12;
  const nameCol = narrow ? Math.min(96, Math.floor(innerW * 0.34)) : Math.min(190, Math.floor(innerW * 0.28));
  const valCol = narrow ? 54 : 76;
  const shareCol = narrow ? 0 : 44;
  const colsUsed = nameCol + valCol + shareCol + gap * (narrow ? 2 : 3);
  const barCol = Math.max(30, innerW - colsUsed);

  const nameX = padX;
  const barX = nameX + nameCol + gap;
  const valueX = barX + barCol + gap;
  const shareX = narrow ? valueX + valCol : valueX + valCol + gap;

  let maxValue = 0;
  for (const part of level.parts) {
    if (positive(part) && part.value! > maxValue) maxValue = part.value!;
  }
  const fill = (part: BreakdownPart) => (positive(part) && maxValue > 0 ? Math.max(2, (part.value! / maxValue) * barCol) : 0);
  const inlineFont = `500 10.5px ${theme.font}`;

  const rows: PartsRowLayout[] = [];
  const items: DrawnItem[] = [];
  let rowY = 0;
  const row = (part: BreakdownPart, parentKey: string | null, depth: BreakdownDepth, rowHeight: number, swatchColor: string, extra: Partial<PartsRowLayout>) => {
    const indent = depth === 0 ? 0 : depth === 1 ? 20 : 34;
    const shareText = shareOf(part, level.total);
    rows.push({
      key: part.key,
      parentKey,
      depth,
      part,
      y: rowY,
      height: rowHeight,
      swatchColor,
      hasChildren: false,
      expanded: false,
      nameX: nameX + indent,
      nameWidth: Math.max(20, nameCol - indent),
      chevronRect: null,
      barX,
      barWidth: barCol,
      barFillWidth: fill(part),
      segments: [],
      valueX,
      valueWidth: valCol,
      shareX,
      shareWidth: shareCol,
      shareText,
      isLastChild: false,
      ...extra,
    });
    const open = extra.hasChildren ? (extra.expanded ? 1 : 0) : -1;
    items.push({ key: part.key, parentKey, depth, part, open, shareText, top: rowY, height: rowHeight });
    rowY += rowHeight;
  };

  for (const part of level.parts) {
    const color = partColour(part, theme);
    const hasChildren = !narrow && rowsBelow(part) > 0;
    const isExpanded = hasChildren && expanded[part.key] === true;
    const barFillWidth = fill(part);
    const segments: PartsSegment[] = [];
    const shown = drawnChildren(part).filter(positive);
    const shares = !isExpanded && barFillWidth > 0 ? subdivision(shown) : null;
    if (shares) {
      const usable = Math.max(0, barFillWidth - Math.max(0, shown.length - 1));
      let segX = barX;
      for (const [at, child] of shown.entries()) {
        const segW = shares[at]! * usable;
        if (segW >= 0.5) {
          const segColor = childTint(color, child, theme, SEGMENT_TINTS);
          const fullText = `${child.name} ${child.text}`;
          let inlineLabel: string | null = null;
          if (!narrow && segW >= 36) {
            if (measure(fullText, inlineFont) <= segW - 10) inlineLabel = fullText;
            else if (measure(child.name, inlineFont) <= segW - 10) inlineLabel = child.name;
          }
          segments.push({ key: child.key, name: child.name, text: child.text, x: segX, width: segW, color: segColor, inlineLabel, ink: contrastInkForFill(segColor, theme), notRecorded: false });
        }
        segX += segW + 1;
      }
    }
    row(part, null, 0, PARTS_ROW_HEIGHT, color, { hasChildren, expanded: isExpanded, segments });
    if (!isExpanded) continue;
    const children = drawnChildren(part);
    children.forEach((child, at) => {
      const grand = drawnChildren(child, false);
      row(child, part.key, 1, PARTS_CHILD_HEIGHT, childTint(color, child, theme, SEGMENT_TINTS), { isLastChild: at === children.length - 1 && grand.length === 0 });
      grand.forEach((item, index) => {
        row(item, child.key, 2, PARTS_ITEM_HEIGHT, childTint(color, child, theme, SEGMENT_TINTS), { isLastChild: at === children.length - 1 && index === grand.length - 1 });
      });
    });
  }

  return { view: 'parts', width, height, narrow, padX, totalLine, bodyTop, rows, contentHeight: bodyTop + rowY + padBottom, expanded, items };
}

function pointInSector(
  px: number,
  py: number,
  cx: number,
  cy: number,
  rIn: number,
  rOut: number,
  a0: number,
  a1: number,
  margin: number,
): boolean {
  const dx = px - cx;
  const dy = py - cy;
  const r = Math.hypot(dx, dy);
  if (r < rIn + margin || r > rOut - margin) return false;
  let ang = Math.atan2(dy, dx);
  while (ang < a0) ang += Math.PI * 2;
  while (ang > a0 + Math.PI * 2) ang -= Math.PI * 2;
  const aMargin = margin / Math.max(r, 1);
  return ang >= a0 + aMargin && ang <= a1 - aMargin;
}

function boxInSector(
  x: number,
  y: number,
  bw: number,
  bh: number,
  cx: number,
  cy: number,
  rIn: number,
  rOut: number,
  a0: number,
  a1: number,
): boolean {
  const hw = bw / 2;
  const hh = bh / 2;
  const points: readonly (readonly [number, number])[] = [
    [x - hw, y - hh],
    [x + hw, y - hh],
    [x - hw, y + hh],
    [x + hw, y + hh],
    [x, y - hh],
    [x, y + hh],
    [x - hw, y],
    [x + hw, y],
  ];
  for (let i = 0; i < points.length; i++) {
    const pt = points[i]!;
    if (!pointInSector(pt[0], pt[1], cx, cy, rIn, rOut, a0, a1, 3)) return false;
  }
  return true;
}

function placeRingInnerLabel(
  name: string,
  shareText: string,
  cx: number,
  cy: number,
  rIn: number,
  rOut: number,
  a0: number,
  a1: number,
  ink: string,
  theme: FamilyTheme,
  measure: MeasureTextFn,
): RingInnerSlice['label'] {
  const mid = (a0 + a1) / 2;
  const rMid = (rIn + rOut) / 2;
  const font1 = `600 12px ${theme.font}`;
  const font2 = `400 11px ${theme.font}`;
  const wName = measure(name, font1);
  const wShare = shareText ? measure(shareText, font2) : 0;

  const tryPlace = (bw: number, bh: number): { x: number; y: number } | null => {
    const drOffsets = [0, -6, 6];
    for (let k = 0; k <= 40; k++) {
      const dist = k * 2.5;
      const dirs = k === 0 ? [0] : [-1, 1];
      for (const dir of dirs) {
        const angle = mid + (dir * dist) / Math.max(rMid, 1);
        if (angle <= a0 || angle >= a1) continue;
        for (const dr of drOffsets) {
          const r = rMid + dr;
          const px = cx + r * Math.cos(angle);
          const py = cy + r * Math.sin(angle);
          if (boxInSector(px, py, bw, bh, cx, cy, rIn, rOut, a0, a1)) {
            return { x: px, y: py };
          }
        }
      }
    }
    return null;
  };

  if (shareText) {
    const twoLinePos = tryPlace(Math.max(wName, wShare), 27);
    if (twoLinePos) {
      return { x: twoLinePos.x, y: twoLinePos.y, line1: name, line2: shareText, ink, fontSize: 12 };
    }
  }
  const oneLinePos = tryPlace(wName, 14);
  if (oneLinePos) {
    return { x: oneLinePos.x, y: oneLinePos.y, line1: name, line2: null, ink, fontSize: 12 };
  }
  return null;
}

function layoutRingView(input: BreakdownLayoutInput): RingViewLayout {
  const { level, periodLabel, comparison, width, height, theme, namesInRing, userExpanded, measure } = input;
  const narrow = width < NARROW_BREAKPOINT;
  const padX = narrow ? 10 : 16;
  const padTop = narrow ? 6 : 8;
  const padBottom = 10;
  const innerW = Math.max(80, width - padX * 2);

  let y = padTop;

  let topLine: RingViewLayout['topLine'] = null;
  if (!narrow) {
    const pieces = [periodLabel, level.total.shortReason].filter((s): s is string => Boolean(s && s.trim()));
    const text = pieces.join(' · ');
    const textRight = padX + measure(text, `400 12px ${theme.font}`);
    const comparisonRect = comparisonBox(comparison, padX + innerW, textRight, y, RING_TOP_LINE_HEIGHT, theme, measure);
    topLine = {
      rect: { x: padX, y, width: innerW, height: RING_TOP_LINE_HEIGHT },
      text,
      ...(comparisonRect && comparison ? { comparison, comparisonRect } : {}),
    };
    y += RING_TOP_LINE_HEIGHT + 4;
  }

  const bodyTop = y;
  const bodyH = Math.max(80, height - bodyTop - padBottom);
  const maxR = narrow ? Math.floor(Math.min(innerW, bodyH) / 2) - 4 : Math.min(132, Math.floor((bodyH - 16) / 2));
  const R = Math.max(44, maxR);
  const w1 = Math.round(0.38 * R);
  const w2 = Math.round(0.17 * R);
  const ringGap = 3;
  const r2o = R;
  const r2i = R - w2;
  const r1o = r2i - ringGap;
  const r1i = Math.max(22, r1o - w1);

  const listGap = 28;
  const showSideList = !narrow && width - (padX * 2 + R * 2 + listGap + 12) >= 180;
  const cx = showSideList ? padX + R + 12 : Math.round(width / 2);
  const cy = bodyTop + Math.round(bodyH / 2);

  const recorded = level.parts.filter(positive);
  let sumRecorded = 0;
  for (const part of recorded) sumRecorded += part.value!;

  const gapRad = recorded.length > 1 ? Math.min(1.5 / Math.max(1, r1o), Math.PI / recorded.length) : 0;
  const usableAngle = (Math.PI * 2 - gapRad * recorded.length) * (sumRecorded > 0 ? sumRecorded / wholeOf(level, sumRecorded) : 1);
  const hatchedRing = level.total.atLeast === true;
  const outerFont = `600 10.5px ${theme.font}`;

  const innerSlices: RingInnerSlice[] = [];
  const outerSlices: RingOuterSlice[] = [];
  let angle = -Math.PI / 2;

  for (const part of recorded) {
    const span = sumRecorded > 0 ? (part.value! / sumRecorded) * usableAngle : 0;
    const a0 = angle;
    const a1 = angle + span;
    angle = a1 + gapRad;

    const color = partColour(part, theme);
    const ink = contrastInkForFill(color, theme);
    const shareText = hatchedRing || part.share == null ? '' : formatShare(part.share);

    let label: RingInnerSlice['label'] = null;
    if (!hatchedRing) {
      if (namesInRing && !narrow) {
        label = placeRingInnerLabel(part.name, shareText, cx, cy, r1i, r1o, a0, a1, ink, theme, measure);
      } else if (span >= 0.5 && shareText) {
        const mid = (a0 + a1) / 2;
        const rm = (r1i + r1o) / 2;
        label = { x: cx + rm * Math.cos(mid), y: cy + rm * Math.sin(mid), line1: shareText, line2: null, ink, fontSize: narrow ? 11 : 12 };
      }
    }
    innerSlices.push({ key: part.key, part, a0, a1, color, hatched: hatchedRing, label });

    const children = drawnChildren(part).filter(positive);
    const shares = subdivision(children);
    if (w2 < 6) continue;
    if (!shares) {
      outerSlices.push({ key: part.key, parentKey: part.key, part, a0, a1, color: part.neutral ? color : tintColour(color, OUTER_TINTS[0]), hatched: hatchedRing, arcLabel: null });
      continue;
    }
    const childGap = children.length > 1 ? Math.min(1 / Math.max(1, r2o), span / (2 * (children.length - 1))) : 0;
    const childUsable = span - childGap * (children.length - 1);
    let bAngle = a0;
    for (const [at, child] of children.entries()) {
      const bSpan = shares[at]! * childUsable;
      const b0 = bAngle;
      const b1 = bAngle + bSpan;
      bAngle = b1 + childGap;
      const chColor = childTint(color, child, theme, OUTER_TINTS);
      let arcLabel: RingOuterSlice['arcLabel'] = null;
      if (!hatchedRing && namesInRing && !narrow && w2 >= 15) {
        const rr = (r2i + r2o) / 2;
        if (measure(child.name, outerFont) <= rr * (b1 - b0) - 8) {
          const mid = (b0 + b1) / 2;
          const norm = ((mid % (Math.PI * 2)) + Math.PI * 2) % (Math.PI * 2);
          arcLabel = { text: child.name, midAngle: mid, reversed: norm > Math.PI * 0.5 && norm < Math.PI * 1.5, ink: contrastInkForFill(chColor, theme) };
        }
      }
      outerSlices.push({ key: child.key, parentKey: part.key, part: child, a0: b0, a1: b1, color: chColor, hatched: hatchedRing, arcLabel });
    }
  }

  const defaultOpen = resolveRingOpenKey(level.parts);
  const expanded: Record<string, boolean> = { ...(defaultOpen ? { [defaultOpen]: true } : {}), ...userExpanded };
  const openKey = level.parts.find(part => expanded[part.key] === true && rowsBelow(part) > 0)?.key ?? null;

  const items: DrawnItem[] = [];
  let sideList: RingViewLayout['sideList'] = null;
  if (showSideList) {
    const listX = cx + R + listGap;
    const listW = Math.min(420, width - padX - listX);
    const listTop = bodyTop + 4;
    const listH = Math.max(40, height - listTop - padBottom);
    const rows: RingSideRow[] = [];
    let ry = 0;
    const push = (part: BreakdownPart, parentKey: string | null, depth: BreakdownDepth, rowHeight: number, swatchColor: string, hasChildren: boolean, isExpanded: boolean) => {
      const shareText = shareOf(part, level.total);
      rows.push({
        key: part.key,
        parentKey,
        depth,
        part,
        y: ry,
        height: rowHeight,
        swatchColor,
        hasChildren,
        expanded: isExpanded,
        chevronRect: hasChildren ? { x: listX, y: ry, width: 14, height: rowHeight } : null,
        shareText,
      });
      items.push({ key: part.key, parentKey, depth, part, open: hasChildren ? (isExpanded ? 1 : 0) : -1, shareText, top: ry, height: rowHeight });
      ry += rowHeight;
    };
    for (const part of level.parts) {
      const color = partColour(part, theme);
      const hasChildren = rowsBelow(part) > 0;
      const isExpanded = hasChildren && part.key === openKey;
      push(part, null, 0, RING_SIDE_ROW_HEIGHT, color, hasChildren, isExpanded);
      if (!isExpanded) continue;
      for (const child of drawnChildren(part)) {
        const tint = childTint(color, child, theme, OUTER_TINTS);
        push(child, part.key, 1, RING_SIDE_ROW_HEIGHT - 1, tint, false, false);
        for (const item of drawnChildren(child, false)) push(item, child.key, 2, RING_SIDE_ROW_HEIGHT - 2, tint, false, false);
      }
    }
    sideList = { x: listX, top: listTop, width: listW, height: listH, rows, contentHeight: ry + 8 };
  } else {
    const outerOf = new Map<string, (typeof outerSlices)[number][]>();
    for (const outer of outerSlices) {
      if (outer.key === outer.parentKey) continue;
      const held = outerOf.get(outer.parentKey);
      if (held) held.push(outer);
      else outerOf.set(outer.parentKey, [outer]);
    }
    for (const slice of innerSlices) {
      items.push({ key: slice.key, parentKey: null, depth: 0, part: slice.part, open: -1, shareText: shareOf(slice.part, level.total) });
      for (const outer of outerOf.get(slice.key) ?? []) items.push({ key: outer.key, parentKey: slice.key, depth: 1, part: outer.part, open: -1, shareText: shareOf(outer.part, level.total) });
    }
    items.push(...undrawnParts(level.parts, new Set(innerSlices.map(slice => slice.key)), level.total));
  }

  const partial = level.total.partial === true || level.total.atLeast === true;
  const centerVerb = narrow ? null : level.total.verb;
  const centerValueSize = narrow || R < 90 ? 16 : 22;
  const undrawn = undrawnNote(level.parts, innerSlices.length, level.total);
  const centerUnitLine = undrawn ?? (narrow ? null : level.total.unit ? (partial ? `${level.total.unit} · Partial` : level.total.unit) : (partial ? 'Partial' : null));
  const valueY = centerVerb ? cy + 4 : cy;
  const valueHalf = measure(level.total.text, `600 ${centerValueSize}px ${theme.font}`) / 2;
  const centerTab = partial && !centerUnitLine ? { x: cx + valueHalf + 5, y: valueY - 6.5, width: 6, height: 13 } : null;

  return {
    view: 'ring',
    width,
    height,
    narrow,
    padX,
    topLine,
    cx,
    cy,
    r1i,
    r1o,
    r2i,
    r2o,
    innerSlices,
    outerSlices,
    center: { verb: centerVerb, value: level.total.text, valueFontSize: centerValueSize, unitLine: centerUnitLine, partialTabRect: centerTab },
    sideList,
    openKey,
    contentHeight: sideList ? sideList.top + sideList.contentHeight + padBottom : height,
    expanded,
    items,
  };
}

interface SquarifyItem {
  readonly part: BreakdownPart;
  readonly weight: number;
  readonly index: number;
}

interface SquarifyRect extends Rect {
  readonly item: SquarifyItem;
}

function worstAspect(row: readonly SquarifyItem[], rowSum: number, shortSide: number, scale: number): number {
  if (!row.length || shortSide <= 0 || rowSum <= 0) return Infinity;
  const s2 = shortSide * shortSide;
  const rowArea = rowSum * scale;
  let maxVal = 0;
  let minVal = Infinity;
  for (const item of row) {
    const a = item.weight * scale;
    if (a > maxVal) maxVal = a;
    if (a < minVal) minVal = a;
  }
  if (minVal <= 0) return Infinity;
  return Math.max((s2 * maxVal) / (rowArea * rowArea), (rowArea * rowArea) / (s2 * minVal));
}

function squarify(items: readonly SquarifyItem[], bounds: Rect, gap: number): SquarifyRect[] {
  const positive = items.filter(i => i.weight > 0);
  if (!positive.length || bounds.width <= 0 || bounds.height <= 0) return [];
  const totalWeight = positive.reduce((s, i) => s + i.weight, 0);
  if (totalWeight <= 0) return [];

  const result: SquarifyRect[] = [];
  let rx = bounds.x;
  let ry = bounds.y;
  let rw = bounds.width;
  let rh = bounds.height;
  let remainingWeight = totalWeight;
  let cursor = 0;

  while (cursor < positive.length && rw > 0.5 && rh > 0.5) {
    const scale = (rw * rh) / remainingWeight;
    const horizontal = rw >= rh;
    const shortSide = horizontal ? rh : rw;

    const row: SquarifyItem[] = [];
    let rowSum = 0;
    let bestWorst = Infinity;

    while (cursor < positive.length) {
      const next = positive[cursor]!;
      const nextSum = rowSum + next.weight;
      const nextWorst = worstAspect([...row, next], nextSum, shortSide, scale);
      if (row.length > 0 && nextWorst > bestWorst) break;
      row.push(next);
      rowSum = nextSum;
      bestWorst = nextWorst;
      cursor++;
    }

    const rowArea = rowSum * scale;
    const thickness = shortSide > 0 ? rowArea / shortSide : 0;
    let offset = 0;
    for (let i = 0; i < row.length; i++) {
      const item = row[i]!;
      const length = rowSum > 0 ? (item.weight / rowSum) * shortSide : 0;
      const rawX = horizontal ? rx : rx + offset;
      const rawY = horizontal ? ry + offset : ry;
      const rawW = horizontal ? thickness : length;
      const rawH = horizontal ? length : thickness;
      const gx = gap / 2;
      const tx = rawX + gx;
      const ty = rawY + gx;
      const tw = Math.max(0, rawW - gap);
      const th = Math.max(0, rawH - gap);
      if (tw > 0.5 && th > 0.5) {
        result.push({ x: tx, y: ty, width: tw, height: th, item });
      }
      offset += length;
    }

    if (horizontal) {
      rx += thickness;
      rw = Math.max(0, rw - thickness);
    } else {
      ry += thickness;
      rh = Math.max(0, rh - thickness);
    }
    remainingWeight -= rowSum;
  }

  return result;
}

const UNALLOCATED: BreakdownPart = { key: '__unallocated__', name: '', ordinal: 0, value: null, text: '' };

function tileLabel(width: number, height: number): MosaicTile['labelMode'] {
  if (width >= 56 && height >= 40) return 'full';
  if (width >= 56 && height >= 22) return 'name';
  return 'none';
}

function layoutMosaicView(input: BreakdownLayoutInput): MosaicViewLayout {
  const { level, periodLabel, comparison, width, height, theme, measure } = input;
  const narrow = width < NARROW_BREAKPOINT;
  const padX = narrow ? 10 : 16;
  const padTop = narrow ? 6 : 8;
  const padBottom = 10;
  const innerW = Math.max(80, width - padX * 2);

  const totalLine = layoutTotalLine(padX, padTop, innerW, narrow, level.total, periodLabel, comparison, theme, measure);
  const top = padTop + TOTAL_LINE_HEIGHT + 4;
  const area: Rect = { x: padX, y: top, width: innerW, height: Math.max(60, height - top - padBottom) };

  const recordedItems: SquarifyItem[] = [];
  level.parts.forEach((part, index) => {
    if (positive(part)) recordedItems.push({ part, weight: part.value!, index });
  });
  const drawnSum = recordedItems.reduce((sum, item) => sum + item.weight, 0);
  const unallocated = wholeOf(level, drawnSum) - drawnSum;
  const placed = unallocated > 0 ? [...recordedItems, { part: UNALLOCATED, weight: unallocated, index: recordedItems.length }] : recordedItems;

  const hatchedAll = level.total.atLeast === true;
  const tiles: MosaicTile[] = [];
  const items: DrawnItem[] = [];

  for (const rect of squarify(placed, area, 3)) {
    if (rect.item.part === UNALLOCATED) continue;
    const part = rect.item.part;
    const baseColor = partColour(part, theme);
    const shareText = hatchedAll || part.share == null ? '' : formatShare(part.share);
    const children = drawnChildren(part).filter(positive);
    const shares = subdivision(children);
    const showGroup = shares !== null && rect.width > 70 && rect.height > 60;
    items.push({ key: part.key, parentKey: null, depth: 0, part, open: -1, shareText });

    if (!showGroup) {
      tiles.push({
        key: part.key,
        parentKey: null,
        part,
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        color: baseColor,
        ink: contrastInkForFill(baseColor, theme),
        hatched: hatchedAll,
        isGroup: false,
        headerHeight: 0,
        headerRight: null,
        labelMode: tileLabel(rect.width, rect.height),
        valueLine: rect.width >= 100 && shareText ? `${part.text} · ${shareText}` : part.text,
        shareText,
        children: [],
      });
      continue;
    }

    const groupColor = shadeColour(baseColor, 0.45);
    const childArea: Rect = {
      x: rect.x + 2,
      y: rect.y + MOSAIC_GROUP_HEADER,
      width: Math.max(0, rect.width - 4),
      height: Math.max(0, rect.height - MOSAIC_GROUP_HEADER - 2),
    };
    const weights: SquarifyItem[] = children.map((child, index) => ({ part: child, weight: shares![index]!, index }));
    const rest = 1 - shares!.reduce((sum, share) => sum + share, 0);
    if (rest > 1e-9) weights.push({ part, weight: rest, index: weights.length });
    const childTiles: MosaicTile[] = [];
    for (const cr of squarify(weights, childArea, 1)) {
      if (cr.item.part === part) continue;
      const child = cr.item.part;
      const chColor = childTint(baseColor, child, theme, TILE_TINTS);
      const chShare = hatchedAll || child.share == null ? '' : formatShare(child.share);
      childTiles.push({
        key: child.key,
        parentKey: part.key,
        part: child,
        x: cr.x,
        y: cr.y,
        width: cr.width,
        height: cr.height,
        color: chColor,
        ink: contrastInkForFill(chColor, theme),
        hatched: hatchedAll,
        isGroup: false,
        headerHeight: 0,
        headerRight: null,
        labelMode: tileLabel(cr.width, cr.height),
        valueLine: cr.width >= 100 && chShare ? `${child.text} · ${chShare}` : child.text,
        shareText: chShare,
        children: [],
      });
      items.push({ key: child.key, parentKey: part.key, depth: 1, part: child, open: -1, shareText: chShare });
    }
    tiles.push({
      key: part.key,
      parentKey: null,
      part,
      x: rect.x,
      y: rect.y,
      width: rect.width,
      height: rect.height,
      color: groupColor,
      ink: contrastInkForFill(groupColor, theme),
      hatched: hatchedAll,
      isGroup: true,
      headerHeight: MOSAIC_GROUP_HEADER,
      headerRight: rect.width > 120 ? (shareText ? `${part.text} · ${shareText}` : part.text) : null,
      labelMode: 'name',
      valueLine: part.text,
      shareText,
      children: childTiles,
    });
  }

  items.push(...undrawnParts(level.parts, new Set(tiles.map(tile => tile.key)), level.total));
  return { view: 'mosaic', width, height, narrow, padX, totalLine, area, tiles, contentHeight: height, expanded: {}, items, undrawn: undrawnNote(level.parts, tiles.length, level.total) };
}

function inRect(r: Rect | null | undefined, x: number, y: number): boolean {
  return !!r && x >= r.x && x <= r.x + r.width && y >= r.y && y <= r.y + r.height;
}

export function firstRowFrom<T extends { readonly y: number; readonly height: number }>(rows: readonly T[], y: number): number {
  let low = 0;
  let high = rows.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (rows[mid]!.y + rows[mid]!.height <= y) low = mid + 1;
    else high = mid;
  }
  return low;
}

function rowAt<T extends { readonly y: number; readonly height: number }>(rows: readonly T[], y: number): T | null {
  const row = rows[firstRowFrom(rows, y)];
  return row && row.y <= y ? row : null;
}

function angleIn(dx: number, dy: number, a0: number, a1: number): boolean {
  let ang = Math.atan2(dy, dx);
  while (ang < a0) ang += Math.PI * 2;
  while (ang > a0 + Math.PI * 2) ang -= Math.PI * 2;
  return ang >= a0 && ang <= a1;
}

export function hitTestBreakdown(layout: BreakdownLayout, x: number, y: number, scrollTop: number): string | null {
  if (layout.view === 'parts') {
    if (inRect(layout.totalLine.partialTabRect, x, y)) return `row:${PARTIAL_KEY}`;
    if (inRect(layout.totalLine.comparisonRect, x, y)) return `row:${COMPARISON_KEY}`;
    if (y < layout.bodyTop) return null;
    const row = rowAt(layout.rows, y - layout.bodyTop + scrollTop);
    if (!row) return null;
    if (row.chevronRect && x >= row.chevronRect.x && x <= row.chevronRect.x + row.chevronRect.width + 6) return `toggle:${row.key}`;
    if (row.segments.length > 0 && x >= row.barX && x <= row.barX + row.barFillWidth) {
      const segment = row.segments.find(seg => x >= seg.x && x <= seg.x + seg.width);
      if (segment) return `row:${segment.key}`;
    }
    return `row:${row.key}`;
  }

  if (layout.view === 'ring') {
    if (layout.topLine && inRect(layout.topLine.comparisonRect, x, y)) return `row:${COMPARISON_KEY}`;
    if (inRect(layout.center.partialTabRect, x, y)) return `row:${PARTIAL_KEY}`;
    const list = layout.sideList;
    if (list && x >= list.x && x <= list.x + list.width && y >= list.top && y <= list.top + list.height) {
      const row = rowAt(list.rows, y - list.top + scrollTop);
      if (row) {
        if (row.chevronRect && x >= row.chevronRect.x && x <= row.chevronRect.x + row.chevronRect.width + 6) return `toggle:${row.key}`;
        return `row:${row.key}`;
      }
    }
    const dx = x - layout.cx;
    const dy = y - layout.cy;
    const r = Math.hypot(dx, dy);
    if (r >= layout.r1i && r <= layout.r1o) {
      const slice = layout.innerSlices.find(item => angleIn(dx, dy, item.a0, item.a1));
      if (slice) return `row:${slice.key}`;
    }
    if (r >= layout.r2i && r <= layout.r2o) {
      const slice = layout.outerSlices.find(item => angleIn(dx, dy, item.a0, item.a1));
      if (slice) return `row:${slice.key}`;
    }
    return null;
  }

  if (inRect(layout.totalLine.partialTabRect, x, y)) return `row:${PARTIAL_KEY}`;
  if (inRect(layout.totalLine.comparisonRect, x, y)) return `row:${COMPARISON_KEY}`;
  for (const tile of layout.tiles) {
    if (!inRect(tile, x, y)) continue;
    const child = tile.isGroup ? tile.children.find(item => inRect(item, x, y)) : undefined;
    return `row:${(child ?? tile).key}`;
  }
  return null;
}
