/** A two-column flow diagram (a Sankey of sources to targets): where each node and
 * ribbon goes, how they glide between playback moments, and what lies under a
 * point. Pure, so it is tested without a browser. The component that draws it is
 * `FlowDiagram.tsx`; widgets hand it nodes and links and keep what they mean. */
import { approach } from '../easing.ts';
import type { CountryFlagLook } from '../../components/CountryFlag';

export type FlowSide = 'source' | 'target';

/** A node: one side's thing (a seller, a buyer). */
export interface FlowNode {
  id: string;
  side: FlowSide;
  label: string;
  color: string;
  /** An image URL drawn before the label, such as a country's flag; null keeps its
   * place with a placeholder, as for a flag not (yet) known; absent, no image. */
  flag?: string | null;
  /** Genuine country slot and sampled read state; aggregates leave this absent. */
  countryFlag?: CountryFlagLook;
  /** A stable key: ribbons inside the other side's nodes keep this order, and equal
   * sizes are ordered by it, so nothing re-orders while playing. */
  order: string;
  /** Always last on its side, whatever its size, such as a node summing the rest. */
  last?: boolean;
}

/** A ribbon from a source node to a target node, `value` wide. */
export interface FlowLink {
  id: string;
  source: string;
  target: string;
  /** Presentation size; exact numbers belong in the widget's tooltip. */
  value: number;
  color: string;
  /** A stable key ordering ribbons between the same two nodes. */
  order: string;
}

export interface FlowInput { nodes: readonly FlowNode[]; links: readonly FlowLink[] }

export interface FlowGeometry {
  /** The columns' inner edges: sources end at `left`, targets start at `right`. */
  left: number; right: number;
  top: number; bottom: number;
  nodeWidth: number;
  gap: number;
  /** Node slots per side the height is shared between, so the scale holds while
   * nodes come and go; at least the most nodes a side has now. */
  slots?: number;
  /** The value that fills the height: one scale for a whole campaign, so ribbons
   * grow and shrink with their amounts. Null fits what is shown now. */
  scale?: number | null;
}

export interface NodeBox { id: string; node: FlowNode; x: number; y: number; width: number; height: number; alpha: number }
export interface LinkShape { id: string; link: FlowLink; x0: number; x1: number; y0: number; y1: number; width: number; alpha: number }
export interface FlowFrame { nodes: NodeBox[]; links: LinkShape[]; settled: boolean }

/** Sizes below this are gone: a vanished ribbon is dropped once it has shrunk away. */
const GONE = 1e-9;
/** The least a node is drawn, in pixels, so a tiny trade is still seen and pointed at. */
export const MIN_NODE = 2;
/** The least gap between two nodes, in pixels. */
export const MIN_GAP = 2;

/** How many nodes a side can show in `height` pixels, each with its own label row
 * (`rowHeight`, the label's line), so no node and no name falls off the diagram.
 * A widget folds the rest into one node (as the race shows "+N more"). */
export function flowCapacity(height: number, rowHeight: number): number {
  return Math.max(2, Math.floor((height + MIN_GAP) / Math.max(1, rowHeight, MIN_NODE + MIN_GAP)));
}

/** An earlier playback extent removes future geometry immediately. */
export const flowPlaybackSnap = (previous: number | undefined, next: number | undefined) => previous !== undefined && next !== undefined && next < previous;

const byOrder = (a: { order: string }, b: { order: string }) => a.order < b.order ? -1 : a.order > b.order ? 1 : 0;

/** Each side's nodes in their slots: largest first, ties by `order`, `last` nodes at
 * the end. Sizes are the larger of what flows in and out. */
export function slotOrder(nodes: readonly FlowNode[], sizes: ReadonlyMap<string, number>, side: FlowSide): FlowNode[] {
  return nodes.filter(node => node.side === side).sort((a, b) => Number(!!a.last) - Number(!!b.last)
    || (sizes.get(b.id) ?? 0) - (sizes.get(a.id) ?? 0) || byOrder(a, b));
}

/** What flows through each node: the sum of its ribbons. */
export function nodeSizes(links: Iterable<{ link: FlowLink; value: number }>): Map<string, number> {
  const sizes = new Map<string, number>();
  for (const { link, value } of links) {
    sizes.set(link.source, (sizes.get(link.source) ?? 0) + value);
    sizes.set(link.target, (sizes.get(link.target) ?? 0) + value);
  }
  return sizes;
}

/** The ribbons' eased sizes and the nodes' eased places, stepped toward each new
 * target. Shapes ease; numbers stay exact: a widget's tooltip reads its own data.
 * A ribbon grows from nothing when it appears and shrinks to nothing when it goes;
 * a node glides to its new slot and fades in and out. `snap` jumps straight to the
 * target, as across a gap in the recording or under reduced motion. */
export class FlowMotion {
  private links = new Map<string, { link: FlowLink; value: number }>();
  private nodes = new Map<string, { node: FlowNode; y: number | null; alpha: number }>();

  /** Advances by `dt` milliseconds with easing time `tau` and lays the diagram out. */
  step(input: FlowInput, geometry: FlowGeometry, dt: number, tau: number, snap = false): FlowFrame {
    const instant = snap || !(tau > 0);
    const move = (current: number, target: number) => instant ? target : approach(current, target, dt, tau);
    let settled = true;
    const wanted = new Map(input.links.map(link => [link.id, link]));
    for (const link of input.links) {
      const eased = this.links.get(link.id);
      const value = move(eased?.value ?? 0, Math.max(0, link.value));
      if (value !== Math.max(0, link.value)) settled = false;
      this.links.set(link.id, { link, value });
    }
    for (const [id, eased] of this.links) {
      if (wanted.has(id)) continue;
      eased.value = move(eased.value, 0);
      if (eased.value <= GONE) this.links.delete(id);
      else settled = false;
    }
    const nodeById = new Map(input.nodes.map(node => [node.id, node]));
    // A ribbon on its way out keeps drawing between its nodes as they were.
    for (const eased of this.links.values()) {
      for (const id of [eased.link.source, eased.link.target]) if (!nodeById.has(id) && this.nodes.has(id)) nodeById.set(id, this.nodes.get(id)!.node);
    }
    const eased = nodeSizes(this.links.values());
    // Slots follow the exact target sizes, so a node heads for where it belongs now.
    const exact = nodeSizes(input.links.map(link => ({ link, value: Math.max(0, link.value) })));
    // A node no ribbon touches has nothing to show: it goes like any other.
    const touched = new Set(input.links.flatMap(link => [link.source, link.target]));
    const present = input.nodes.filter(node => nodeById.get(node.id) === node && touched.has(node.id));
    const columns = (['source', 'target'] as const).map(side => slotOrder(present, exact, side));
    const slots = Math.max(1, geometry.slots ?? 0, ...columns.map(column => column.length));
    // Gaps give way before nodes do: never more than a quarter of the height between
    // them, never under `MIN_GAP`; every node keeps `MIN_NODE` however small it is,
    // so none vanishes. A widget asks for no more nodes than `flowCapacity` fits.
    const height = geometry.bottom - geometry.top;
    const gap = slots > 1 ? Math.max(MIN_GAP, Math.min(geometry.gap, height / 4 / (slots - 1))) : geometry.gap;
    const room = Math.max(1, height - gap * (slots - 1) - MIN_NODE * slots);
    // What a side needs: every node at the larger of its eased and target size (as
    // slots are spaced below), so nothing runs past the bottom mid-glide.
    const needs = Math.max(...(['source', 'target'] as const).map(side => [...nodeById.values()]
      .reduce((sum, node) => sum + (node.side === side ? Math.max(eased.get(node.id) ?? 0, exact.get(node.id) ?? 0) : 0), 0)));
    const full = Math.max(geometry.scale ?? 0, needs, GONE);
    const k = room / full;
    const boxes = new Map<string, NodeBox>();
    for (const [side, column] of (['source', 'target'] as const).map((side, index) => [side, columns[index]] as const)) {
      let y = geometry.top;
      const x = side === 'source' ? geometry.left - geometry.nodeWidth : geometry.right;
      for (const node of column) {
        const state = this.nodes.get(node.id) ?? { node, y: null, alpha: 0 };
        state.node = node;
        state.y = state.y == null ? y : move(state.y, y);
        state.alpha = move(state.alpha, 1);
        if (state.y !== y || state.alpha !== 1) settled = false;
        this.nodes.set(node.id, state);
        boxes.set(node.id, { id: node.id, node, x, y: state.y, width: geometry.nodeWidth, height: MIN_NODE + (eased.get(node.id) ?? 0) * k, alpha: state.alpha });
        // Room for the larger of its size now and where it is heading, so a node
        // shrinking or growing never overlaps the next.
        y += MIN_NODE + Math.max(exact.get(node.id) ?? 0, eased.get(node.id) ?? 0) * k + gap;
      }
    }
    // Nodes no longer wanted fade where they are while their ribbons shrink away.
    for (const [id, state] of this.nodes) {
      if (boxes.has(id)) continue;
      const size = eased.get(id) ?? 0;
      state.alpha = move(state.alpha, 0);
      if (size <= GONE && state.alpha <= 0.01) { this.nodes.delete(id); continue; }
      settled = false;
      const x = state.node.side === 'source' ? geometry.left - geometry.nodeWidth : geometry.right;
      boxes.set(id, { id, node: state.node, x, y: state.y ?? geometry.top, width: geometry.nodeWidth, height: MIN_NODE * state.alpha + size * k, alpha: state.alpha });
    }
    // Inside a node, ribbons keep a fixed order: by the other end's key (a `last` node
    // last, as it is placed), then their own.
    const order = (id: string) => nodeById.get(id)?.order ?? id;
    const last = (id: string) => Number(!!nodeById.get(id)?.last);
    const ribbons = [...this.links.values()];
    const offsets = new Map<string, number>();
    const ends = new Map<string, { y0: number; y1: number }>();
    const place = (end: 'source' | 'target', other: 'source' | 'target') => {
      const sorted = [...ribbons].sort((a, b) => last(a.link[other]) - last(b.link[other])
        || byOrder({ order: order(a.link[other]) }, { order: order(b.link[other]) }) || byOrder(a.link, b.link) || (a.link.id < b.link.id ? -1 : 1));
      for (const { link, value } of sorted) {
        const node = boxes.get(link[end]);
        const offset = offsets.get(`${end}:${link[end]}`) ?? 0;
        offsets.set(`${end}:${link[end]}`, offset + value * k);
        const y = (node?.y ?? geometry.top) + offset;
        const current = ends.get(link.id) ?? { y0: 0, y1: 0 };
        if (end === 'source') current.y0 = y; else current.y1 = y;
        ends.set(link.id, current);
      }
    };
    place('source', 'target');
    place('target', 'source');
    const links: LinkShape[] = ribbons.map(({ link, value }) => {
      const at = ends.get(link.id)!;
      const alpha = Math.min(boxes.get(link.source)?.alpha ?? 0, boxes.get(link.target)?.alpha ?? 0);
      return { id: link.id, link, x0: geometry.left, x1: geometry.right, y0: at.y0, y1: at.y1, width: value * k, alpha };
    });
    return { nodes: [...boxes.values()], links, settled };
  }

  /** Forgets every shape, so the next step starts afresh. */
  reset(): void { this.links.clear(); this.nodes.clear(); }
}

/** A ribbon's upper edge at `x`: the cubic curve from (x0, y0) to (x1, y1) with
 * both control points halfway across, the curve `drawRibbon` traces. */
export function ribbonTop(shape: Pick<LinkShape, 'x0' | 'x1' | 'y0' | 'y1'>, x: number): number | null {
  const span = shape.x1 - shape.x0;
  if (!(span > 0) || x < shape.x0 || x > shape.x1) return null;
  const along = (t: number) => 1.5 * t * (1 - t) + t * t * t;
  let low = 0;
  let high = 1;
  const goal = (x - shape.x0) / span;
  for (let step = 0; step < 30; step++) {
    const middle = (low + high) / 2;
    if (along(middle) < goal) low = middle; else high = middle;
  }
  const t = (low + high) / 2;
  return shape.y0 + (shape.y1 - shape.y0) * (3 * t * t - 2 * t * t * t);
}

/** Pixels either side of a thin ribbon or node that still count as on it. */
export const HIT_SLOP = 3;

/** The ribbon under a point: the topmost drawn (the last in `links`) whose band,
 * widened to be easy to point at, holds it. */
export function linkAt(links: readonly LinkShape[], x: number, y: number): LinkShape | null {
  for (let index = links.length - 1; index >= 0; index--) {
    const shape = links[index];
    if (shape.alpha < 0.05) continue;
    const top = ribbonTop(shape, x);
    if (top == null) continue;
    const slop = Math.max(0, (HIT_SLOP * 2 - shape.width) / 2);
    if (y >= top - slop && y <= top + shape.width + slop) return shape;
  }
  return null;
}

/** The node under a point, its box widened to be easy to point at. */
export function nodeAt(nodes: readonly NodeBox[], x: number, y: number): NodeBox | null {
  for (const box of nodes) {
    if (box.alpha < 0.05) continue;
    const slop = Math.max(0, (HIT_SLOP * 2 - box.height) / 2);
    if (x >= box.x - HIT_SLOP && x <= box.x + box.width + HIT_SLOP && y >= box.y - slop && y <= box.y + box.height + slop) return box;
  }
  return null;
}

/** Where a node's flag and name were drawn, beside its bar. */
export interface LabelBox { id: string; node: FlowNode; x: number; y: number; width: number; height: number; alpha: number }

/** The node whose drawn flag or name holds a point. */
export function labelAt(labels: readonly LabelBox[], x: number, y: number): LabelBox | null {
  for (const box of labels) {
    if (box.alpha < 0.05) continue;
    if (x >= box.x && x <= box.x + box.width && y >= box.y && y <= box.y + box.height) return box;
  }
  return null;
}

/** What the keyboard stops at, in reading order: each source node and then its
 * ribbons (in their order inside it), then each target node. */
export type FlowStop = { kind: 'node'; id: string } | { kind: 'link'; id: string };

export function flowStops(frame: Pick<FlowFrame, 'nodes' | 'links'>): FlowStop[] {
  const shown = (box: { alpha: number }) => box.alpha > 0.5;
  const nodes = frame.nodes.filter(shown).sort((a, b) => a.y - b.y);
  const stops: FlowStop[] = [];
  const links = new Map<string, LinkShape[]>();
  for (const shape of frame.links) {
    if (!(shape.width > 0) || !shown(shape)) continue;
    const source = shape.link.source;
    const ribbons = links.get(source) ?? [];
    ribbons.push(shape); links.set(source, ribbons);
  }
  for (const box of nodes.filter(box => box.node.side === 'source')) {
    stops.push({ kind: 'node', id: box.id });
    for (const shape of (links.get(box.id) ?? []).sort((a, b) => a.y0 - b.y0)) stops.push({ kind: 'link', id: shape.id });
  }
  for (const box of nodes.filter(box => box.node.side === 'target')) stops.push({ kind: 'node', id: box.id });
  return stops;
}
