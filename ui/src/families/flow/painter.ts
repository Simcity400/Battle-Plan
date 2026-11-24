import { LABEL_WEIGHT, VALUE_WEIGHT } from '../../charts/draw.ts';
import { ellipsise } from '../../charts/labels.ts';
import { colour } from '../../charts/palette.ts';
import { textWidth, type Canvas2D } from '../../charts/textWidth.ts';
import { drawFlagSlot, drawLegendStrip, FLAG_SLOT_HEIGHT, FLAG_SLOT_WIDTH, resolveKitTokens } from '../../draw/kit.ts';
import { resolveCachedArt, toFlagSlotState } from '../art/adapters.ts';
import type { LaneArtCache } from '../art/laneCache.ts';
import type { ArtResult, FamilyTheme } from '../contract.ts';
import { drawHatchRect } from '../valueCanvas.ts';
import type { FlowModel, FlowNode, FlowRibbon } from './model.ts';

export const FLOW_PAD = 12;
export const FLOW_BAR_WIDTH = 6;
export const FLOW_CENTRE_WIDTH = 8;
export const FLOW_LEGEND_HEIGHT = 22;
export const FLOW_HEADER_HEIGHT = 16;
export const FLOW_CENTRE_LABEL_HEIGHT = 16;
export const FLOW_NODE_GAP = 6;
export const FLOW_LABEL_MIN = 14;
export const FLOW_MISSING_HEIGHT = 6;
const SAMPLES = 20;
const FLAG_GAP = 6;

export interface FlowNodeBox {
  readonly node: FlowNode;
  readonly x: number;
  readonly y: number;
  readonly height: number;
  readonly slotTop: number;
  readonly slotHeight: number;
}

export interface FlowRibbonBox {
  readonly ribbon: FlowRibbon;
  readonly x0: number;
  readonly x1: number;
  readonly a0: number;
  readonly a1: number;
  readonly b0: number;
  readonly b1: number;
}

export interface FlowLayout {
  readonly width: number;
  readonly height: number;
  readonly legendHeight: number;
  readonly headerTop: number;
  readonly bodyTop: number;
  readonly labelWidth: number;
  readonly leftBarX: number;
  readonly rightBarX: number;
  readonly centreX: number;
  readonly nodes: readonly FlowNodeBox[];
  readonly ribbons: readonly FlowRibbonBox[];
  readonly scale: number;
}

const ease = (u: number) => u * u * (3 - 2 * u);

function heightOf(value: number | null, scale: number): number {
  if (value === null) return FLOW_MISSING_HEIGHT;
  if (value <= 0) return 0;
  return Math.max(2, value * scale);
}

function solveScale(sides: readonly (readonly FlowNode[])[], available: number, extra: number): number {
  let best = Infinity;
  for (const nodes of sides) {
    if (!nodes.length) continue;
    const gaps = FLOW_NODE_GAP * (nodes.length - 1);
    let fixed = 0;
    let total = 0;
    for (const node of nodes) {
      if (node.numeric === null) fixed += Math.max(FLOW_MISSING_HEIGHT, FLOW_LABEL_MIN);
      else total += Math.max(0, node.numeric);
    }
    if (total <= 0) continue;
    let scale = Math.max(0, (available - gaps - fixed) / total);
    for (let pass = 0; pass < 3; pass++) {
      let small = 0;
      let large = 0;
      for (const node of nodes) {
        if (node.numeric === null) continue;
        const h = heightOf(node.numeric, scale);
        if (h < FLOW_LABEL_MIN) small += FLOW_LABEL_MIN;
        else large += Math.max(0, node.numeric);
      }
      if (large <= 0) break;
      scale = Math.max(0, (available - gaps - fixed - small) / large);
    }
    best = Math.min(best, scale);
  }
  if (extra > 0) best = Math.min(best, available / extra);
  return Number.isFinite(best) ? best : 0;
}

function stack(nodes: readonly FlowNode[], x: number, top: number, scale: number): FlowNodeBox[] {
  const out: FlowNodeBox[] = [];
  let cursor = top;
  for (const node of nodes) {
    const height = heightOf(node.numeric, scale);
    const slotHeight = Math.max(height, FLOW_LABEL_MIN);
    out.push({ node, x, y: cursor + (slotHeight - height) / 2, height, slotTop: cursor, slotHeight });
    cursor += slotHeight + FLOW_NODE_GAP;
  }
  return out;
}

export function layoutFlow(model: FlowModel, width: number, height: number): FlowLayout {
  const w = Math.max(1, Math.floor(width));
  const h = Math.max(1, Math.floor(height));
  const legendHeight = model.parts.length > 1 || model.notRecordedKey ? FLOW_LEGEND_HEIGHT : 0;
  const headerTop = legendHeight;
  const hasHeader = Boolean(model.leftHeader || model.rightHeader || model.unitNote);
  const centred = model.mode === 'centred' && model.centre !== undefined;
  const bodyTop = headerTop + (hasHeader ? FLOW_HEADER_HEIGHT : 0) + 4 + (centred ? FLOW_CENTRE_LABEL_HEIGHT : 0);
  const available = Math.max(20, h - bodyTop - FLOW_PAD / 2);
  const labelWidth = Math.max(70, Math.min(210, Math.round(w * 0.22)));
  const leftBarX = FLOW_PAD + labelWidth;
  const rightBarX = w - FLOW_PAD - labelWidth - FLOW_BAR_WIDTH;
  const centreX = Math.round((leftBarX + FLOW_BAR_WIDTH + rightBarX) / 2 - FLOW_CENTRE_WIDTH / 2);

  const sumOf = (nodes: readonly FlowNode[]) => nodes.reduce((sum, node) => sum + Math.max(0, node.numeric ?? 0), 0);
  const centreExtra = centred ? Math.max(sumOf(model.left), sumOf(model.right)) : 0;
  const scale = solveScale([model.left, model.right], available, centreExtra);
  const left = stack(model.left, leftBarX, bodyTop, scale);
  const right = stack(model.right, rightBarX, bodyTop, scale);
  const nodes: FlowNodeBox[] = [...left, ...right];
  const ribbons: FlowRibbonBox[] = [];
  const widthOf = (ribbon: FlowRibbon) => heightOf(ribbon.numeric, scale);

  if (centred) {
    const span = (boxes: readonly FlowNodeBox[]) => boxes.length ? boxes[boxes.length - 1]!.slotTop + boxes[boxes.length - 1]!.slotHeight - bodyTop : 0;
    const inbound = model.ribbons.filter(ribbon => ribbon.to === model.centre!.key);
    const outbound = model.ribbons.filter(ribbon => ribbon.from === model.centre!.key);
    const inHeight = inbound.reduce((sum, ribbon) => sum + widthOf(ribbon), 0);
    const outHeight = outbound.reduce((sum, ribbon) => sum + widthOf(ribbon), 0);
    const centreHeight = Math.max(inHeight, outHeight, 2);
    const centreTop = bodyTop + Math.max(0, (Math.max(span(left), span(right)) - centreHeight) / 2);
    nodes.push({ node: model.centre!, x: centreX, y: centreTop, height: centreHeight, slotTop: centreTop - FLOW_CENTRE_LABEL_HEIGHT, slotHeight: centreHeight + FLOW_CENTRE_LABEL_HEIGHT });
    const byKey = new Map(nodes.map(box => [box.node.key, box]));
    let leftCursor = centreTop + (centreHeight - inHeight) / 2;
    for (const ribbon of inbound) {
      const from = byKey.get(ribbon.from);
      if (!from) continue;
      const rw = widthOf(ribbon);
      ribbons.push({ ribbon, x0: from.x + FLOW_BAR_WIDTH, x1: centreX, a0: from.y, a1: from.y + Math.min(rw, from.height || rw), b0: leftCursor, b1: leftCursor + rw });
      leftCursor += rw;
    }
    let rightCursor = centreTop + (centreHeight - outHeight) / 2;
    for (const ribbon of outbound) {
      const to = byKey.get(ribbon.to);
      if (!to) continue;
      const rw = widthOf(ribbon);
      ribbons.push({ ribbon, x0: centreX + FLOW_CENTRE_WIDTH, x1: to.x, a0: rightCursor, a1: rightCursor + rw, b0: to.y, b1: to.y + Math.min(rw, to.height || rw) });
      rightCursor += rw;
    }
  } else {
    const byKey = new Map(nodes.map(box => [box.node.key, box]));
    const outCursor = new Map<string, number>();
    const inCursor = new Map<string, number>();
    const order = new Map(right.map((box, index) => [box.node.key, index]));
    const sorted = [...model.ribbons].sort((a, b) => (order.get(a.to) ?? 0) - (order.get(b.to) ?? 0));
    for (const ribbon of sorted) {
      const from = byKey.get(ribbon.from);
      const to = byKey.get(ribbon.to);
      if (!from || !to) continue;
      const rw = widthOf(ribbon);
      const a = outCursor.get(from.node.key) ?? from.y;
      const b = inCursor.get(to.node.key) ?? to.y;
      ribbons.push({ ribbon, x0: from.x + FLOW_BAR_WIDTH, x1: to.x, a0: a, a1: a + rw, b0: b, b1: b + rw });
      outCursor.set(from.node.key, a + rw);
      inCursor.set(to.node.key, b + rw);
    }
  }
  return { width: w, height: h, legendHeight, headerTop, bodyTop, labelWidth, leftBarX, rightBarX, centreX, nodes, ribbons, scale };
}

function edgeAt(u: number, start: number, end: number): number {
  return start + (end - start) * ease(u);
}

function ribbonPath(ctx: Canvas2D, box: FlowRibbonBox, from: number, to: number): void {
  const span = Math.max(1, box.x1 - box.x0);
  const aTop = box.a0 + (box.a1 - box.a0) * from;
  const aBottom = box.a0 + (box.a1 - box.a0) * to;
  const bTop = box.b0 + (box.b1 - box.b0) * from;
  const bBottom = box.b0 + (box.b1 - box.b0) * to;
  ctx.beginPath();
  for (let i = 0; i <= SAMPLES; i++) {
    const u = i / SAMPLES;
    const x = box.x0 + span * u;
    const y = edgeAt(u, aTop, bTop);
    if (i === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  for (let i = SAMPLES; i >= 0; i--) {
    const u = i / SAMPLES;
    const y = edgeAt(u, aBottom, bBottom);
    ctx.lineTo(box.x0 + span * u, y);
  }
  ctx.closePath();
}

export function hitFlow(layout: FlowLayout, x: number, y: number): string | null {
  for (const box of layout.nodes) {
    const node = box.node;
    let inside: boolean;
    if (node.side === 'left') inside = x >= 0 && x <= box.x + FLOW_BAR_WIDTH + 1 && y >= box.slotTop && y <= box.slotTop + box.slotHeight;
    else if (node.side === 'right') inside = x >= box.x - 1 && x <= layout.width && y >= box.slotTop && y <= box.slotTop + box.slotHeight;
    else inside = x >= box.x - 4 && x <= box.x + FLOW_CENTRE_WIDTH + 4 && y >= box.slotTop && y <= box.y + box.height;
    if (inside) return `node:${node.key}`;
  }
  for (let i = layout.ribbons.length - 1; i >= 0; i--) {
    const box = layout.ribbons[i]!;
    if (x < box.x0 || x > box.x1) continue;
    const u = (x - box.x0) / Math.max(1, box.x1 - box.x0);
    const s = ease(u);
    const top = box.a0 + (box.b0 - box.a0) * s;
    const bottom = box.a1 + (box.b1 - box.a1) * s;
    if (y >= top - 1 && y <= bottom + 1) return `ribbon:${box.ribbon.key}`;
  }
  return null;
}

export interface DrawFlowInput {
  readonly model: FlowModel;
  readonly layout: FlowLayout;
  readonly theme: FamilyTheme;
  readonly inspected: string | null;
  readonly focused: string | null;
  readonly artCache?: LaneArtCache;
}

function flagState(art: ArtResult | undefined, tag: string | undefined, cache?: LaneArtCache) {
  if (!art) return { kind: 'missing' as const, tag: tag ?? '?' };
  return toFlagSlotState(resolveCachedArt(art, { artCache: cache }), tag);
}

function drawNodeLabel(ctx: Canvas2D, box: FlowNodeBox, layout: FlowLayout, theme: FamilyTheme, artCache: LaneArtCache | undefined, highlighted: boolean): void {
  const node = box.node;
  const middle = box.y + box.height / 2;
  const fontSize = theme.fontSize.small;
  ctx.textBaseline = 'middle';
  ctx.font = `${VALUE_WEIGHT} ${fontSize}px ${theme.font}`;
  const figure = node.figure;
  const figureW = figure ? textWidth(ctx, figure) : 0;
  const flagW = node.others !== undefined ? 0 : FLAG_SLOT_WIDTH;
  const room = Math.max(10, layout.labelWidth - figureW - (flagW ? flagW + FLAG_GAP : 0) - 12);
  ctx.font = `${LABEL_WEIGHT} ${fontSize}px ${theme.font}`;
  const name = ellipsise(node.label, room, s => textWidth(ctx, s));
  const nameW = textWidth(ctx, name);
  const muted = node.labelMissing || node.others !== undefined;
  const nameColour = muted ? theme.muted : theme.text;
  if (node.side === 'left') {
    let cursor = box.x - 6;
    ctx.textAlign = 'right';
    ctx.font = `${VALUE_WEIGHT} ${fontSize}px ${theme.font}`;
    ctx.fillStyle = node.numeric === null ? theme.muted : theme.text;
    if (figure) ctx.fillText(figure, cursor, middle + 0.5);
    cursor -= figureW + (figure ? 6 : 0);
    ctx.font = `${highlighted ? VALUE_WEIGHT : LABEL_WEIGHT} ${fontSize}px ${theme.font}`;
    ctx.fillStyle = nameColour;
    ctx.fillText(name, cursor, middle + 0.5);
    cursor -= nameW + FLAG_GAP;
    if (flagW) drawFlagSlot(ctx, cursor - FLAG_SLOT_WIDTH, Math.round(middle - FLAG_SLOT_HEIGHT / 2), flagState(node.flag, node.tag, artCache), theme);
  } else if (node.side === 'right') {
    let cursor = box.x + FLOW_BAR_WIDTH + 6;
    if (flagW) {
      drawFlagSlot(ctx, cursor, Math.round(middle - FLAG_SLOT_HEIGHT / 2), flagState(node.flag, node.tag, artCache), theme);
      cursor += FLAG_SLOT_WIDTH + FLAG_GAP;
    }
    ctx.textAlign = 'left';
    ctx.font = `${highlighted ? VALUE_WEIGHT : LABEL_WEIGHT} ${fontSize}px ${theme.font}`;
    ctx.fillStyle = nameColour;
    ctx.fillText(name, cursor, middle + 0.5);
    cursor += nameW + 6;
    ctx.font = `${VALUE_WEIGHT} ${fontSize}px ${theme.font}`;
    ctx.fillStyle = node.numeric === null ? theme.muted : theme.text;
    if (figure) ctx.fillText(figure, cursor, middle + 0.5);
  } else {
    ctx.textAlign = 'center';
    ctx.textBaseline = 'bottom';
    ctx.font = `${VALUE_WEIGHT} ${fontSize}px ${theme.font}`;
    ctx.fillStyle = theme.text;
    const centreRoom = Math.max(40, layout.rightBarX - layout.leftBarX - 20);
    ctx.fillText(ellipsise(node.label, centreRoom, s => textWidth(ctx, s)), box.x + FLOW_CENTRE_WIDTH / 2, box.y - 3);
  }
}

export function drawFlow(ctx: Canvas2D, input: DrawFlowInput): void {
  const { model, layout, theme, inspected, focused, artCache } = input;
  const tokens = resolveKitTokens(theme);
  ctx.save();
  ctx.fillStyle = theme.surface;
  ctx.fillRect(0, 0, layout.width, layout.height);

  if (layout.legendHeight > 0) {
    const entries = [
      ...(model.parts.length > 1
        ? model.parts.map(part => ({ label: part.label, color: colour(theme.surface, part.ordinal) }))
        : []),
      ...(model.notRecordedKey ? [{ label: 'Not recorded', color: '', hatch: true, muted: true }] : []),
    ];
    if (entries.length) {
      drawLegendStrip(ctx, 0, 0, layout.width, layout.legendHeight, {
        entries,
        padX: FLOW_PAD,
        entryGap: 12,
      }, theme);
    }
  }

  const headerMiddle = layout.headerTop + FLOW_HEADER_HEIGHT / 2;
  ctx.font = `${LABEL_WEIGHT} ${theme.fontSize.small}px ${theme.font}`;
  ctx.fillStyle = theme.muted;
  ctx.textBaseline = 'middle';
  if (model.leftHeader) {
    ctx.textAlign = 'left';
    ctx.fillText(model.leftHeader, FLOW_PAD, headerMiddle);
  }
  if (model.unitNote) {
    ctx.textAlign = 'center';
    ctx.fillText(model.unitNote, layout.centreX + FLOW_CENTRE_WIDTH / 2, headerMiddle);
  }
  if (model.rightHeader) {
    ctx.textAlign = 'right';
    ctx.fillText(model.rightHeader, layout.width - FLOW_PAD, headerMiddle);
  }

  const hoveredRibbon = inspected?.startsWith('ribbon:') ? inspected.slice(7) : null;
  const hoveredNode = inspected?.startsWith('node:') ? inspected.slice(5) : null;
  const lit = (ribbon: FlowRibbon) => hoveredRibbon ? ribbon.key === hoveredRibbon : hoveredNode ? ribbon.from === hoveredNode || ribbon.to === hoveredNode : null;
  for (const box of layout.ribbons) {
    const ribbon = box.ribbon;
    const on = lit(ribbon);
    const alpha = on === null ? 0.62 : on ? 0.9 : 0.16;
    if (ribbon.numeric === null) {
      ctx.save();
      ctx.globalAlpha = on === false ? 0.3 : 1;
      ribbonPath(ctx, box, 0, 1);
      ctx.clip();
      drawHatchRect(ctx, box.x0, Math.min(box.a0, box.b0) - 1, box.x1 - box.x0, Math.max(box.a1, box.b1) - Math.min(box.a0, box.b0) + 2, 0, theme);
      ctx.restore();
      continue;
    }
    const total = ribbon.bands.reduce((sum, band) => sum + Math.max(0, band.numeric), 0);
    if (total <= 0) continue;
    let from = 0;
    ctx.globalAlpha = alpha;
    for (const band of ribbon.bands) {
      const share = Math.max(0, band.numeric) / total;
      if (share <= 0) continue;
      const part = model.parts[band.part];
      ctx.fillStyle = colour(theme.surface, part?.ordinal ?? band.part);
      ribbonPath(ctx, box, from, from + share);
      ctx.fill();
      from += share;
    }
    ctx.globalAlpha = 1;
  }

  for (const box of layout.nodes) {
    const node = box.node;
    const barWidth = node.side === 'centre' ? FLOW_CENTRE_WIDTH : FLOW_BAR_WIDTH;
    if (node.numeric === null) {
      drawHatchRect(ctx, box.x, box.y, barWidth, box.height, 1, theme);
    } else if (box.height > 0) {
      ctx.fillStyle = node.others !== undefined ? tokens.barMuted : node.side === 'centre' ? theme.text : theme.muted;
      ctx.beginPath();
      ctx.roundRect(box.x, box.y, barWidth, box.height, 1.5);
      ctx.fill();
    }
    const highlighted = hoveredNode === node.key || focused === `node:${node.key}`;
    drawNodeLabel(ctx, box, layout, theme, artCache, highlighted);
    if (focused === `node:${node.key}`) {
      ctx.strokeStyle = theme.accent;
      ctx.lineWidth = 1.5;
      ctx.strokeRect(node.side === 'left' ? 2 : box.x - 2, box.slotTop - 1, node.side === 'left' ? box.x + barWidth : layout.width - box.x, box.slotHeight + 2);
    }
  }
  ctx.restore();
}
