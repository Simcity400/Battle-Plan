import { contrastInkForFill } from '../../draw/kit.ts';
import { shadeColour, tintColour } from '../../shell/appearance/color.ts';
import type { FamilyTheme } from '../contract.ts';
import { formatShare } from '../valueFormat.ts';
import type { BreakdownViewMode } from './options.ts';
import {
  partColour,
  resolveExpandedMap,
  resolveRingOpenKey,
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

export interface TotalLineLayout {
  readonly rect: Rect;
  readonly narrow: boolean;
  readonly total: BreakdownTotal;
  readonly periodLabel?: string;
  readonly comparison?: BreakdownComparison;
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
  readonly depth: 0 | 1;
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
  readonly depth: 0 | 1;
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
  readonly expanded: Readonly<Record<string, boolean>>;
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
const RING_SIDE_ROW_HEIGHT = 20;
const RING_TOP_LINE_HEIGHT = 28;
const MOSAIC_GROUP_HEADER = 20;

export function layoutBreakdown(input: BreakdownLayoutInput): BreakdownLayout {
  if (input.view === 'ring') return layoutRingView(input);
  if (input.view === 'mosaic') return layoutMosaicView(input);
  return layoutPartsView(input);
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
  if (total.unit) {
    cursor += 4 + measure(total.unit, `400 12px ${theme.font}`);
  }
  let partialTabRect: Rect | undefined;
  if (total.atLeast || total.partial) {
    const tabX = cursor + 6;
    partialTabRect = { x: tabX, y: y + Math.round((TOTAL_LINE_HEIGHT - 13) / 2), width: 6, height: 13 };
  }
  return {
    rect: { x, y, width, height: TOTAL_LINE_HEIGHT },
    narrow,
    total,
    periodLabel: narrow ? undefined : periodLabel,
    comparison: narrow ? undefined : comparison,
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

  let y = padTop;

  const totalLine = layoutTotalLine(padX, y, innerW, narrow, level.total, periodLabel, comparison, theme, measure);
  y += TOTAL_LINE_HEIGHT + 4;
  const bodyTop = y;

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
    if (!part.notRecorded && part.value !== null && part.value > maxValue) {
      maxValue = part.value;
    }
  }
  const inlineFont = `500 10.5px ${theme.font}`;

  const rows: PartsRowLayout[] = [];
  let rowY = 0;

  for (const part of level.parts) {
    const color = partColour(part, theme);
    const hasChildren = Boolean(!part.notRecorded && part.children && part.children.length > 0);
    const isExpanded = hasChildren && !narrow && expanded[part.key] === true;
    const barFillWidth = !part.notRecorded && part.value !== null && maxValue > 0
      ? Math.max(part.value > 0 ? 2 : 0, (part.value / maxValue) * barCol)
      : 0;
    const chevronRect: Rect | null = null;

    const segments: PartsSegment[] = [];
    if (hasChildren && !isExpanded && barFillWidth > 0 && part.children) {
      const orderedChildren = part.children;
      let childSum = 0;
      for (const ch of orderedChildren) {
        if (!ch.notRecorded && ch.value !== null && ch.value > 0) childSum += ch.value;
      }
      if (childSum > 0) {
        let segX = barX;
        const segGap = 1;
        const positive = orderedChildren.filter(ch => !ch.notRecorded && ch.value !== null && ch.value > 0);
        const usableW = Math.max(0, barFillWidth - segGap * Math.max(0, positive.length - 1));
        for (let i = 0; i < positive.length; i++) {
          const ch = positive[i]!;
          const segW = (ch.value! / childSum) * usableW;
          if (segW < 0.5) continue;
          const segColor = i % 2 === 0 ? color : tintColour(color, 0.2);
          const ink = contrastInkForFill(segColor, theme);
          const fullText = `${ch.name} ${ch.text}`;
          let inlineLabel: string | null = null;
          if (!narrow && segW >= 36) {
            if (measure(fullText, inlineFont) <= segW - 10) inlineLabel = fullText;
            else if (measure(ch.name, inlineFont) <= segW - 10) inlineLabel = ch.name;
          }
          segments.push({
            key: ch.key,
            name: ch.name,
            text: ch.text,
            x: segX,
            width: segW,
            color: segColor,
            inlineLabel,
            ink,
            notRecorded: false,
          });
          segX += segW + segGap;
        }
      }
    }

    const leadOffset = 0;
    rows.push({
      key: part.key,
      parentKey: null,
      depth: 0,
      part,
      y: rowY,
      height: PARTS_ROW_HEIGHT,
      swatchColor: color,
      hasChildren,
      expanded: isExpanded,
      nameX: nameX + leadOffset,
      nameWidth: Math.max(20, nameCol - leadOffset),
      chevronRect,
      barX,
      barWidth: barCol,
      barFillWidth,
      segments,
      valueX,
      valueWidth: valCol,
      shareX,
      shareWidth: shareCol,
      shareText: level.total.atLeast || part.notRecorded ? '' : formatShare(part.share),
      isLastChild: false,
    });
    rowY += PARTS_ROW_HEIGHT;

    if (isExpanded && part.children) {
      const children = part.children;
      for (let ci = 0; ci < children.length; ci++) {
        const ch = children[ci]!;
        const chFillWidth = !ch.notRecorded && ch.value !== null && maxValue > 0
          ? Math.max(ch.value > 0 ? 2 : 0, (ch.value / maxValue) * barCol)
          : 0;
        const chColor = color;
        rows.push({
          key: ch.key,
          parentKey: part.key,
          depth: 1,
          part: ch,
          y: rowY,
          height: PARTS_CHILD_HEIGHT,
          swatchColor: chColor,
          hasChildren: false,
          expanded: false,
          nameX: nameX + 20,
          nameWidth: Math.max(20, nameCol - 20),
          chevronRect: null,
          barX,
          barWidth: barCol,
          barFillWidth: chFillWidth,
          segments: [],
          valueX,
          valueWidth: valCol,
          shareX,
          shareWidth: shareCol,
          shareText: level.total.atLeast || ch.notRecorded ? '' : formatShare(ch.share),
          isLastChild: ci === children.length - 1,
        });
        rowY += PARTS_CHILD_HEIGHT;
      }
    }
  }

  return {
    view: 'parts',
    width,
    height,
    narrow,
    padX,
    totalLine,
    bodyTop,
    rows,
    contentHeight: rowY + padBottom,
    expanded,
  };
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
    const partialTabRect = undefined;
    topLine = {
      rect: { x: padX, y, width: innerW, height: RING_TOP_LINE_HEIGHT },
      text,
      comparison,
      partialTabRect,
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

  const recorded = level.parts.filter(p => !p.notRecorded && p.value !== null && p.value > 0);
  let sumRecorded = 0;
  for (const part of recorded) sumRecorded += part.value!;

  const gapRad = recorded.length > 1 ? 1.5 / Math.max(1, r1o) : 0;
  const usableAngle = Math.max(0.01, Math.PI * 2 - gapRad * recorded.length);
  const hatchedRing = level.total.atLeast === true;
  const outerFont = `600 10.5px ${theme.font}`;

  const innerSlices: RingInnerSlice[] = [];
  const outerSlices: RingOuterSlice[] = [];
  let angle = -Math.PI / 2;

  for (const part of recorded) {
    const frac = sumRecorded > 0 ? part.value! / sumRecorded : 0;
    const span = frac * usableAngle;
    const a0 = angle;
    const a1 = angle + span;
    angle = a1 + gapRad;

    const color = partColour(part, theme);
    const ink = contrastInkForFill(color, theme);
    const shareText = hatchedRing ? '' : formatShare(part.share);

    let label: RingInnerSlice['label'] = null;
    if (!hatchedRing) {
      if (namesInRing && !narrow) {
        label = placeRingInnerLabel(part.name, shareText, cx, cy, r1i, r1o, a0, a1, ink, theme, measure);
      } else if (span >= 0.5 && shareText) {
        const mid = (a0 + a1) / 2;
        const rm = (r1i + r1o) / 2;
        label = {
          x: cx + rm * Math.cos(mid),
          y: cy + rm * Math.sin(mid),
          line1: shareText,
          line2: null,
          ink,
          fontSize: narrow ? 11 : 12,
        };
      }
    }

    innerSlices.push({
      key: part.key,
      part,
      a0,
      a1,
      color,
      hatched: hatchedRing,
      label,
    });

    if (part.children && part.children.length > 0 && w2 >= 6) {
      const positiveChildren = part.children.filter(ch => !ch.notRecorded && ch.value !== null && ch.value > 0);
      let childSum = 0;
      for (const ch of positiveChildren) childSum += ch.value!;
      if (childSum > 0) {
        const childGap = positiveChildren.length > 1 ? 1 / Math.max(1, r2o) : 0;
        const childUsable = Math.max(0.001, span - childGap * Math.max(0, positiveChildren.length - 1));
        let bAngle = a0;
        for (let ci = 0; ci < positiveChildren.length; ci++) {
          const ch = positiveChildren[ci]!;
          const bSpan = (ch.value! / childSum) * childUsable;
          const b0 = bAngle;
          const b1 = bAngle + bSpan;
          bAngle = b1 + childGap;
          const chColor = ci % 2 === 0 ? tintColour(color, 0.12) : tintColour(color, 0.30);
          const chInk = contrastInkForFill(chColor, theme);
          let arcLabel: RingOuterSlice['arcLabel'] = null;
          if (!hatchedRing && namesInRing && !narrow && w2 >= 15) {
            const rr = (r2i + r2o) / 2;
            const tw = measure(ch.name, outerFont);
            if (tw <= rr * (b1 - b0) - 8) {
              const mid = (b0 + b1) / 2;
              const norm = ((mid % (Math.PI * 2)) + Math.PI * 2) % (Math.PI * 2);
              const reversed = norm > Math.PI * 0.5 && norm < Math.PI * 1.5;
              arcLabel = { text: ch.name, midAngle: mid, reversed, ink: chInk };
            }
          }
          outerSlices.push({
            key: ch.key,
            parentKey: part.key,
            part: ch,
            a0: b0,
            a1: b1,
            color: chColor,
            hatched: hatchedRing,
            arcLabel,
          });
        }
      }
    }
  }

  const defaultOpen = resolveRingOpenKey(level.parts);
  const expanded: Record<string, boolean> = { ...(defaultOpen ? { [defaultOpen]: true } : {}), ...userExpanded };
  const openKey = level.parts.find(part => expanded[part.key] === true && part.children?.length)?.key ?? null;

  let sideList: RingViewLayout['sideList'] = null;
  if (showSideList) {
    const listX = cx + R + listGap;
    const listW = Math.min(420, width - padX - listX);
    const listTop = bodyTop + 4;
    const listH = Math.max(40, height - listTop - padBottom);
    const rows: RingSideRow[] = [];
    let ry = 0;
    for (const part of level.parts) {
      const color = partColour(part, theme);
      const hasChildren = Boolean(!part.notRecorded && part.children && part.children.length > 0);
      const isExpanded = hasChildren && expanded[part.key] === true;
      rows.push({
        key: part.key,
        parentKey: null,
        depth: 0,
        part,
        y: ry,
        height: RING_SIDE_ROW_HEIGHT,
        swatchColor: color,
        hasChildren,
        expanded: isExpanded,
        chevronRect: hasChildren ? { x: listX, y: ry, width: 14, height: RING_SIDE_ROW_HEIGHT } : null,
        shareText: hatchedRing || part.notRecorded ? '' : formatShare(part.share),
      });
      ry += RING_SIDE_ROW_HEIGHT;
      if (isExpanded && part.children) {
        const children = part.children;
        for (let ci = 0; ci < children.length; ci++) {
          const ch = children[ci]!;
          const chColor = ci % 2 === 0 ? tintColour(color, 0.12) : tintColour(color, 0.30);
          rows.push({
            key: ch.key,
            parentKey: part.key,
            depth: 1,
            part: ch,
            y: ry,
            height: RING_SIDE_ROW_HEIGHT - 1,
            swatchColor: chColor,
            hasChildren: false,
            expanded: false,
            chevronRect: null,
            shareText: hatchedRing || ch.notRecorded ? '' : formatShare(ch.share),
          });
          ry += RING_SIDE_ROW_HEIGHT - 1;
        }
      }
    }
    sideList = {
      x: listX,
      top: listTop,
      width: listW,
      height: listH,
      rows,
      contentHeight: ry + 8,
    };
  }

  const centerVerb = narrow ? null : level.total.verb;
  const centerValueSize = narrow || R < 90 ? 16 : 22;
  const centerUnitLine = narrow
    ? null
    : level.total.unit
      ? (level.total.partial ? `${level.total.unit} · Partial` : level.total.unit)
      : (level.total.partial ? 'Partial' : null);

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
    center: {
      verb: centerVerb,
      value: level.total.text,
      valueFontSize: centerValueSize,
      unitLine: centerUnitLine,
    },
    sideList,
    openKey,
    expanded,
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

function layoutMosaicView(input: BreakdownLayoutInput): MosaicViewLayout {
  const { level, periodLabel, comparison, width, height, theme, measure } = input;
  const narrow = width < NARROW_BREAKPOINT;
  const padX = narrow ? 10 : 16;
  const padTop = narrow ? 6 : 8;
  const padBottom = 10;
  const innerW = Math.max(80, width - padX * 2);

  let y = padTop;

  const totalLine = layoutTotalLine(padX, y, innerW, narrow, level.total, periodLabel, comparison, theme, measure);
  y += TOTAL_LINE_HEIGHT + 4;

  const area: Rect = {
    x: padX,
    y,
    width: innerW,
    height: Math.max(60, height - y - padBottom),
  };

  const recordedItems: SquarifyItem[] = [];
  level.parts.forEach((part, index) => {
    if (!part.notRecorded && part.value !== null && part.value > 0) {
      recordedItems.push({ part, weight: part.value, index });
    }
  });

  const hatchedAll = level.total.atLeast === true;
  const outerRects = squarify(recordedItems, area, 3);
  const tiles: MosaicTile[] = [];

  for (const rect of outerRects) {
    const part = rect.item.part;
    const baseColor = partColour(part, theme);
    const shareText = hatchedAll ? '' : formatShare(part.share);
    const positiveChildren = part.children
      ? part.children.filter(ch => !ch.notRecorded && ch.value !== null && ch.value > 0)
      : [];
    const showGroup = positiveChildren.length > 0 && rect.width > 70 && rect.height > 60;

    if (showGroup) {
      const groupColor = shadeColour(baseColor, 0.45);
      const groupInk = contrastInkForFill(groupColor, theme);
      const childArea: Rect = {
        x: rect.x + 2,
        y: rect.y + MOSAIC_GROUP_HEADER,
        width: Math.max(0, rect.width - 4),
        height: Math.max(0, rect.height - MOSAIC_GROUP_HEADER - 2),
      };
      const childShades = [baseColor, tintColour(baseColor, 0.14), shadeColour(baseColor, 0.14), tintColour(baseColor, 0.26)];
      const childRects = squarify(
        positiveChildren.map((ch, idx) => ({ part: ch, weight: ch.value!, index: idx })),
        childArea,
        1,
      );
      const childTiles: MosaicTile[] = childRects.map((cr, idx) => {
        const chColor = childShades[idx % childShades.length]!;
        const chShare = hatchedAll ? '' : formatShare(cr.item.part.share);
        const labelMode = cr.width >= 56 && cr.height >= 40
          ? 'full'
          : cr.width >= 56 && cr.height >= 22
            ? 'name'
            : 'none';
        const valueLine = cr.width >= 100 && chShare ? `${cr.item.part.text} · ${chShare}` : cr.item.part.text;
        return {
          key: cr.item.part.key,
          parentKey: part.key,
          part: cr.item.part,
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
          labelMode,
          valueLine,
          shareText: chShare,
          children: [],
        };
      });
      const headerRight = rect.width > 120
        ? (shareText ? `${part.text} · ${shareText}` : part.text)
        : null;
      tiles.push({
        key: part.key,
        parentKey: null,
        part,
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        color: groupColor,
        ink: groupInk,
        hatched: hatchedAll,
        isGroup: true,
        headerHeight: MOSAIC_GROUP_HEADER,
        headerRight,
        labelMode: 'name',
        valueLine: part.text,
        shareText,
        children: childTiles,
      });
    } else {
      const labelMode = rect.width >= 56 && rect.height >= 40
        ? 'full'
        : rect.width >= 56 && rect.height >= 22
          ? 'name'
          : 'none';
      const valueLine = rect.width >= 100 && shareText ? `${part.text} · ${shareText}` : part.text;
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
        labelMode,
        valueLine,
        shareText,
        children: [],
      });
    }
  }

  return {
    view: 'mosaic',
    width,
    height,
    narrow,
    padX,
    totalLine,
    area,
    tiles,
    contentHeight: height,
    expanded: {},
  };
}

function inRect(r: Rect | null | undefined, x: number, y: number): boolean {
  return !!r && x >= r.x && x <= r.x + r.width && y >= r.y && y <= r.y + r.height;
}

export function hitTestBreakdown(layout: BreakdownLayout, x: number, y: number, scrollTop: number): string | null {
  if (layout.view === 'parts') {
    if (inRect(layout.totalLine.partialTabRect, x, y)) return 'row:__partial__';
    if (y < layout.bodyTop) return null;
    const localY = y - layout.bodyTop + scrollTop;
    for (const row of layout.rows) {
      if (localY >= row.y && localY < row.y + row.height) {
        if (row.chevronRect && x >= row.chevronRect.x && x <= row.chevronRect.x + row.chevronRect.width + 6) {
          return `toggle:${row.key}`;
        }
        if (row.segments.length > 0 && x >= row.barX && x <= row.barX + row.barFillWidth) {
          for (const seg of row.segments) {
            if (x >= seg.x && x <= seg.x + seg.width) {
              return `row:${seg.key}`;
            }
          }
        }
        return `row:${row.key}`;
      }
    }
    return null;
  }

  if (layout.view === 'ring') {
    if (layout.topLine && inRect(layout.topLine.partialTabRect, x, y)) return 'row:__partial__';
    if (layout.sideList && x >= layout.sideList.x && x <= layout.sideList.x + layout.sideList.width && y >= layout.sideList.top && y <= layout.sideList.top + layout.sideList.height) {
      const localY = y - layout.sideList.top + scrollTop;
      for (const row of layout.sideList.rows) {
        if (localY >= row.y && localY < row.y + row.height) {
          if (row.chevronRect && x >= row.chevronRect.x && x <= row.chevronRect.x + row.chevronRect.width + 6) {
            return `toggle:${row.key}`;
          }
          return `row:${row.key}`;
        }
      }
    }
    const dx = x - layout.cx;
    const dy = y - layout.cy;
    const r = Math.hypot(dx, dy);
    if (r >= layout.r1i && r <= layout.r1o) {
      for (const slice of layout.innerSlices) {
        let ang = Math.atan2(dy, dx);
        while (ang < slice.a0) ang += Math.PI * 2;
        while (ang > slice.a0 + Math.PI * 2) ang -= Math.PI * 2;
        if (ang >= slice.a0 && ang <= slice.a1) return `row:${slice.key}`;
      }
    }
    if (r >= layout.r2i && r <= layout.r2o) {
      for (const slice of layout.outerSlices) {
        let ang = Math.atan2(dy, dx);
        while (ang < slice.a0) ang += Math.PI * 2;
        while (ang > slice.a0 + Math.PI * 2) ang -= Math.PI * 2;
        if (ang >= slice.a0 && ang <= slice.a1) return `row:${slice.key}`;
      }
    }
    return null;
  }

  if (inRect(layout.totalLine.partialTabRect, x, y)) return 'row:__partial__';
  for (const tile of layout.tiles) {
    if (x >= tile.x && x <= tile.x + tile.width && y >= tile.y && y <= tile.y + tile.height) {
      if (tile.isGroup) {
        for (const ch of tile.children) {
          if (x >= ch.x && x <= ch.x + ch.width && y >= ch.y && y <= ch.y + ch.height) {
            return `row:${ch.key}`;
          }
        }
      }
      return `row:${tile.key}`;
    }
  }
  return null;
}
