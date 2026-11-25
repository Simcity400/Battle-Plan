import { LABEL_WEIGHT, VALUE_WEIGHT } from '../../charts/draw.ts';
import { ellipsise } from '../../charts/labels.ts';
import { colour } from '../../charts/palette.ts';
import { textWidth, type Canvas2D } from '../../charts/textWidth.ts';
import { drawFlagSlot, drawLegendStrip, FLAG_SLOT_HEIGHT, FLAG_SLOT_WIDTH, resolveKitTokens, type ArtSlotState } from '../../draw/kit.ts';
import { resolveCachedArt, toArtSlotState } from '../art/adapters.ts';
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
const FLOW_OTHERS_ALPHA = 0.45;
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
  readonly known: number;
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

function knownOf(ribbon: FlowRibbon): number {
  return ribbon.bands.reduce((sum, band) => sum + Math.max(0, band.numeric), 0);
}

function ribbonWidth(ribbon: FlowRibbon, scale: number): number {
  if (!ribbon.missing) return heightOf(ribbon.numeric, scale);
  const known = knownOf(ribbon);
  return (known > 0 ? Math.max(2, known * scale) : 0) + FLOW_MISSING_HEIGHT;
}

function knownShare(ribbon: FlowRibbon, width: number): number {
  return ribbon.missing && width > 0 ? Math.max(0, width - FLOW_MISSING_HEIGHT) / width : 1;
}

function nodeHeight(node: FlowNode, scale: number, touching: ReadonlyMap<string, readonly FlowRibbon[]>): number {
  const ribbons = touching.get(node.key) ?? [];
  const widths = ribbons.reduce((sum, ribbon) => sum + ribbonWidth(ribbon, scale), 0);
  return node.numeric !== null ? Math.max(heightOf(node.numeric, scale), widths) : Math.max(FLOW_MISSING_HEIGHT, widths);
}

function sideExtent(nodes: readonly FlowNode[], scale: number, touching: ReadonlyMap<string, readonly FlowRibbon[]>): number {
  if (!nodes.length) return 0;
  return nodes.reduce((sum, node) => sum + Math.max(nodeHeight(node, scale, touching), FLOW_LABEL_MIN), FLOW_NODE_GAP * (nodes.length - 1));
}

function stack(nodes: readonly FlowNode[], x: number, top: number, scale: number, touching: ReadonlyMap<string, readonly FlowRibbon[]>): FlowNodeBox[] {
  const out: FlowNodeBox[] = [];
  let cursor = top;
  for (const node of nodes) {
    const height = nodeHeight(node, scale, touching);
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
  const viewport = Math.max(20, h - bodyTop - FLOW_PAD / 2);
  const labelWidth = Math.max(70, Math.min(210, Math.round(w * 0.22)));
  const leftBarX = FLOW_PAD + labelWidth;
  const rightBarX = w - FLOW_PAD - labelWidth - FLOW_BAR_WIDTH;
  const centreX = Math.round((leftBarX + FLOW_BAR_WIDTH + rightBarX) / 2 - FLOW_CENTRE_WIDTH / 2);

  const touching = new Map<string, FlowRibbon[]>();
  for (const ribbon of model.ribbons) {
    for (const end of [ribbon.from, ribbon.to]) {
      if (end === model.centre?.key) continue;
      const held = touching.get(end);
      if (held) held.push(ribbon);
      else touching.set(end, [ribbon]);
    }
  }
  const inbound = centred ? model.ribbons.filter(ribbon => ribbon.to === model.centre!.key) : [];
  const outbound = centred ? model.ribbons.filter(ribbon => ribbon.from === model.centre!.key) : [];
  const widthsOf = (ribbons: readonly FlowRibbon[], scale: number) => ribbons.reduce((sum, ribbon) => sum + ribbonWidth(ribbon, scale), 0);
  const extent = (scale: number) => Math.max(
    sideExtent(model.left, scale, touching),
    sideExtent(model.right, scale, touching),
    widthsOf(inbound, scale),
    widthsOf(outbound, scale),
  );
  const knownSide = (nodes: readonly FlowNode[]) => nodes.reduce((sum, node) => sum + (node.numeric !== null ? Math.max(0, node.numeric) : (touching.get(node.key) ?? []).reduce((held, ribbon) => held + knownOf(ribbon), 0)), 0);
  const most = Math.max(knownSide(model.left), knownSide(model.right));
  const floor = extent(0);
  const available = floor > viewport ? floor + viewport : viewport;
  let scale = 0;
  if (most > 0) {
    let low = 0;
    let high = available / most;
    for (let pass = 0; pass < 40; pass++) {
      const middle = (low + high) / 2;
      if (extent(middle) <= available) low = middle;
      else high = middle;
    }
    scale = low;
  }
  const left = stack(model.left, leftBarX, bodyTop, scale, touching);
  const right = stack(model.right, rightBarX, bodyTop, scale, touching);
  const nodes: FlowNodeBox[] = [...left, ...right];
  const ribbons: FlowRibbonBox[] = [];
  const widthOf = (ribbon: FlowRibbon) => ribbonWidth(ribbon, scale);
  const contentHeight = Math.ceil(bodyTop + extent(scale) + FLOW_PAD / 2);

  if (centred) {
    const span = (boxes: readonly FlowNodeBox[]) => boxes.length ? boxes[boxes.length - 1]!.slotTop + boxes[boxes.length - 1]!.slotHeight - bodyTop : 0;
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
      ribbons.push({ ribbon, x0: from.x + FLOW_BAR_WIDTH, x1: centreX, a0: from.y, a1: from.y + Math.min(rw, from.height || rw), b0: leftCursor, b1: leftCursor + rw, known: knownShare(ribbon, rw) });
      leftCursor += rw;
    }
    let rightCursor = centreTop + (centreHeight - outHeight) / 2;
    for (const ribbon of outbound) {
      const to = byKey.get(ribbon.to);
      if (!to) continue;
      const rw = widthOf(ribbon);
      ribbons.push({ ribbon, x0: centreX + FLOW_CENTRE_WIDTH, x1: to.x, a0: rightCursor, a1: rightCursor + rw, b0: to.y, b1: to.y + Math.min(rw, to.height || rw), known: knownShare(ribbon, rw) });
      rightCursor += rw;
    }
  } else {
    const byKey = new Map(nodes.map(box => [box.node.key, box]));
    const outCursor = new Map<string, number>();
    const inCursor = new Map<string, number>();
    for (const ribbon of model.ribbons) {
      const from = byKey.get(ribbon.from);
      const to = byKey.get(ribbon.to);
      if (!from || !to) continue;
      const rw = widthOf(ribbon);
      const a = outCursor.get(from.node.key) ?? from.y;
      const b = inCursor.get(to.node.key) ?? to.y;
      ribbons.push({ ribbon, x0: from.x + FLOW_BAR_WIDTH, x1: to.x, a0: a, a1: a + rw, b0: b, b1: b + rw, known: knownShare(ribbon, rw) });
      outCursor.set(from.node.key, a + rw);
      inCursor.set(to.node.key, b + rw);
    }
  }
  return { width: w, height: Math.max(h, contentHeight), legendHeight, headerTop, bodyTop, labelWidth, leftBarX, rightBarX, centreX, nodes, ribbons, scale };
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
    if (x < box.x0 || x > box.x1 || (box.a1 <= box.a0 && box.b1 <= box.b0)) continue;
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
  readonly scrollTop?: number;
}

function flagState(art: ArtResult | undefined, tag: string | undefined, cache?: LaneArtCache): ArtSlotState {
  return art ? toArtSlotState(resolveCachedArt(art, cache), tag) : { kind: 'none' };
}

function drawNodeLabel(ctx: Canvas2D, box: FlowNodeBox, layout: FlowLayout, theme: FamilyTheme, artCache: LaneArtCache | undefined, highlighted: boolean): void {
  const node = box.node;
  const middle = box.y + box.height / 2;
  const fontSize = theme.fontSize.small;
  ctx.textBaseline = 'middle';
  ctx.font = `${VALUE_WEIGHT} ${fontSize}px ${theme.font}`;
  const figure = node.figure;
  const figureW = figure ? textWidth(ctx, figure) : 0;
  const flagW = node.others !== undefined || node.unknown !== undefined ? 0 : FLAG_SLOT_WIDTH;
  const room = Math.max(10, layout.labelWidth - figureW - (flagW ? flagW + FLAG_GAP : 0) - 12);
  ctx.font = `${LABEL_WEIGHT} ${fontSize}px ${theme.font}`;
  const name = ellipsise(node.label, room, s => textWidth(ctx, s));
  const nameW = textWidth(ctx, name);
  const muted = node.labelMissing || node.labelLoading || node.others !== undefined;
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
  const { model, layout, theme, inspected, focused, artCache, scrollTop = 0 } = input;
  const tokens = resolveKitTokens(theme);
  ctx.save();
  ctx.translate(0, -scrollTop);
  ctx.fillStyle = theme.surface;
  ctx.fillRect(0, 0, layout.width, layout.height);

  if (layout.legendHeight > 0) {
    const partCount = model.parts.length > 1 ? model.parts.length : 0;
    const trailing = model.notRecordedKey ? [{ label: 'Not recorded', color: '', hatch: true, muted: true }] : undefined;
    if (partCount > 0 || trailing) {
      drawLegendStrip(ctx, 0, 0, layout.width, layout.legendHeight, {
        entries: {
          length: partCount,
          at: i => {
            const part = model.parts[i]!;
            return { label: part.label, color: colour(theme.surface, part.ordinal), swatch: 'square' as const };
          },
          ...(trailing ? { trailing } : {}),
        },
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

  const lens = inspected ?? focused;
  const hoveredRibbon = lens?.startsWith('ribbon:') ? lens.slice(7) : null;
  const hoveredNode = lens?.startsWith('node:') ? lens.slice(5) : null;
  const lit = (ribbon: FlowRibbon) => hoveredRibbon ? ribbon.key === hoveredRibbon : hoveredNode ? ribbon.from === hoveredNode || ribbon.to === hoveredNode : null;
  for (const box of layout.ribbons) {
    const ribbon = box.ribbon;
    const on = lit(ribbon);
    const alpha = on === null ? 0.62 : on ? 0.9 : 0.16;
    const total = knownOf(ribbon);
    const knownTo = ribbon.missing ? box.known : 1;
    if (total > 0 && knownTo > 0) {
      ctx.globalAlpha = alpha;
      if (ribbon.others) {
        ctx.globalAlpha = alpha * FLOW_OTHERS_ALPHA;
        ctx.fillStyle = tokens.barNeutral;
        ribbonPath(ctx, box, 0, knownTo);
        ctx.fill();
      } else {
        let from = 0;
        for (const band of ribbon.bands) {
          const share = (Math.max(0, band.numeric) / total) * knownTo;
          if (share <= 0) continue;
          const part = model.parts[band.part];
          ctx.fillStyle = colour(theme.surface, part?.ordinal ?? band.part);
          ribbonPath(ctx, box, from, from + share);
          ctx.fill();
          from += share;
        }
      }
      ctx.globalAlpha = 1;
    }
    if (ribbon.missing && knownTo < 1) {
      ctx.save();
      ctx.globalAlpha = on === false ? 0.3 : 1;
      ribbonPath(ctx, box, knownTo, 1);
      ctx.clip();
      drawHatchRect(ctx, box.x0, Math.min(box.a0, box.b0) - 1, box.x1 - box.x0, Math.max(box.a1, box.b1) - Math.min(box.a0, box.b0) + 2, 0, theme);
      ctx.restore();
    }
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
