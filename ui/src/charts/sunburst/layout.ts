import { mix } from '../../shell/appearance/color.ts';
import type { Rgb } from '../../shell/appearance/color';
import { CATEGORY_LIMIT } from '../palette.ts';
import type { TooltipContent, TooltipRow } from '../tooltip';

/** The sunburst's pure parts (spec 2.4, mock-up style-3): a tree grouped from a
 * widget's items, folded so no parent has more slices than the palette has colours,
 * partitioned into rings around the slice the view is zoomed into, coloured,
 * hit-tested and stepped through with the keys. Nothing here draws. */

/** One level's key of an item: what the slice is (its identity within its parent)
 * and how it is named and pictured, from the game or the recording only. */
export interface SunburstKey { id: string; label: string; icon?: string | null }

/** One thing a sunburst adds up, such as one kind of project in one state. */
export interface SunburstItem {
  /** One key per level, innermost first. */
  path: readonly SunburstKey[];
  /** Its amount at the moment shown, for geometry; null when it is not measured
   * (`note` says why). */
  value: number | null;
  /** The same amount exactly, for what is written (`SunburstText.format`), where a
   * number could lose digits: an integer in the widget's own units. */
  exact?: bigint | null;
  /** The total cannot be told, rather than merely not measured. Unknown parts
   * make every ancestor's total unknown, even beside exact measured parts. */
  unknownValue?: boolean;
  /** Incomplete attribution at particular path levels. The exact leaf can stay
   * known even when its type or domain cannot have a complete total. */
  unknownLevels?: readonly boolean[];
  /** False for an item that has not appeared by the moment shown: it adds only its
   * `final`, so the order, the colours and Fill's whole stay those of the end. */
  present?: boolean;
  /** Its whole amount at the end of what is shown: Fill growth sweeps toward it, and
   * slices keep one order while playback grows them. Defaults to `value`. */
  final?: number | null;
  /** The whole total cannot be told. Any numeric `final` is only an ordering
   * weight; Fill shows Share until its denominator is known. */
  finalUnknown?: boolean;
  /** Incomplete attribution of the whole at particular path levels. */
  finalUnknownLevels?: readonly boolean[];
  /** Parts of it not counted by the moment shown, as counts by reason. */
  unknown?: Readonly<Record<string, number>>;
  /** Why it is not measured, when `value` is null. */
  note?: string;
  /** The things it holds by the moment shown (such as projects), by id, named by
   * `SunburstText.countLabel`: a node counts each id once, however many of its
   * items hold it (a project that repaired and then built, with the kind level off). */
  counted?: readonly string[];
}

/** An independently established whole, when it does not depend on how its parts
 * are attributed (such as all deliveries despite an unresolved equipment type). */
export type SunburstTotal = Pick<SunburstItem, 'value' | 'exact' | 'unknownValue' | 'final' | 'finalUnknown' | 'note'>;

export interface SunburstNode {
  /** Unique in the tree: the keys from the root, so it survives regrouping. */
  id: string;
  key: string;
  label: string;
  icon?: string;
  depth: number;
  parent: SunburstNode | null;
  children: SunburstNode[];
  /** The measured amount: its own items' and its children's. */
  value: number;
  /** The same exactly, when every measured item gave one; else null. */
  exact: bigint | null;
  valueUnknown: boolean;
  final: number;
  finalUnknown: boolean;
  /** Something in it has appeared by the moment shown. */
  present: boolean;
  /** Nothing in it is measured: it has no slice, and the legend says why. */
  unmeasured: boolean;
  /** Some present part of it is not measured (such as a type's repairs beside
   * its new buildings, with the kind level off): it stays listed even when its
   * measured amount is zero. */
  partlyUnmeasured: boolean;
  /** Why parts of it are not measured, each once. */
  notes: string[];
  /** Parts not counted by the moment shown, by reason. */
  unknown: Record<string, number>;
  /** The things its items hold, by id; `count` is how many. */
  counted: Set<string>;
  count: number;
  /** The slice that holds the smallest of more siblings than there are colours. */
  other?: boolean;
}

const SEPARATOR = '\u0000';
const add = (into: Record<string, number>, from: Readonly<Record<string, number>> | undefined) => {
  for (const [reason, count] of Object.entries(from ?? {})) if (count > 0) into[reason] = (into[reason] ?? 0) + count;
};

/** Whether a node can be drawn at some moment of what is shown: it has a whole
 * amount. Folding and palette slots go by this alone, so they hold all playback long. */
export const drawable = (node: SunburstNode) => node.final > 0 || node.value > 0;

/** Whether a node is listed (legend, read-out): it has appeared, and it has an
 * amount, is not measured, or has parts not counted. A node with no amount yet but
 * with parts not counted stays listed, with its uncertainty, though it has no slice. */
export const listed = (node: SunburstNode) => node.present && (node.valueUnknown || node.unmeasured || node.partlyUnmeasured || node.value > 0 || unknownTotal(node) > 0);

/** Whether a node shows "Not measured" rather than an amount: none of it is
 * measured, or part of it is not and the rest has no amount yet. */
export const notMeasured = (node: SunburstNode) => node.unmeasured || (node.partlyUnmeasured && !(node.value > 0));

/** The order of a node's children, stable while playback grows them: Other last,
 * then by whole amount (`final`, so parts never drawn come after), then by data key;
 * never by the amount so far or by what has appeared, which change with playback. */
function order(a: SunburstNode, b: SunburstNode): number {
  return Number(a.other ?? false) - Number(b.other ?? false)
    || b.final - a.final || (a.key < b.key ? -1 : a.key > b.key ? 1 : 0);
}

/** The tree of `items` under a root, with only the levels `shown` (a level set to
 * false is skipped, so its children join up and items with the same keys below it
 * add together). Amounts and unknown counts add up the tree; a node is unmeasured
 * only when every item in it is. */
export function groupTree(items: readonly SunburstItem[], root: { id: string; label: string }, shown: readonly boolean[] = [], authoritativeTotal?: SunburstTotal): SunburstNode {
  const top: SunburstNode = { id: root.id, key: root.id, label: root.label, depth: 0, parent: null, children: [], value: 0, exact: 0n, valueUnknown: false, final: 0, finalUnknown: false, present: false, unmeasured: true, partlyUnmeasured: false, notes: [], unknown: {}, counted: new Set(), count: 0 };
  const byId = new Map<string, SunburstNode>([[top.id, top]]);
  const measured = new Set<SunburstNode>();
  const notes = new Map<SunburstNode, Set<string>>();
  for (const item of items) {
    const keys = item.path.map((key, level) => ({ key, level })).filter(({ level }) => shown[level] ?? true);
    let node = top;
    const chain = [top];
    const uncertain = new Set<SunburstNode>();
    const finalUncertain = new Set<SunburstNode>();
    for (const { key, level } of keys) {
      const id = `${node.id}${SEPARATOR}${key.id}`;
      let child = byId.get(id);
      if (!child) {
        child = { id, key: key.id, label: key.label, icon: key.icon ?? undefined, depth: node.depth + 1, parent: node, children: [], value: 0, exact: 0n, valueUnknown: false, final: 0, finalUnknown: false, present: false, unmeasured: true, partlyUnmeasured: false, notes: [], unknown: {}, counted: new Set(), count: 0 };
        byId.set(id, child);
        node.children.push(child);
      }
      node = child;
      chain.push(child);
      if (item.unknownLevels?.[level] && item.present !== false) for (const link of chain) uncertain.add(link);
      if (item.finalUnknownLevels?.[level]) for (const link of chain) finalUncertain.add(link);
    }
    const present = item.present !== false;
    const value = present ? item.value : null;
    const final = item.final === undefined ? item.value : item.final;
    for (const link of chain) {
      if (final != null) link.final += final;
      link.finalUnknown ||= item.finalUnknown === true || finalUncertain.has(link);
      if (!present) continue;
      link.present = true;
      link.valueUnknown ||= item.unknownValue === true || uncertain.has(link);
      if (value != null) {
        link.value += value;
        link.exact = link.exact != null && item.exact != null ? link.exact + item.exact : null;
        measured.add(link);
      }
      add(link.unknown, item.unknown);
      for (const id of item.counted ?? []) link.counted.add(id);
      link.count = link.counted.size;
      if (value == null) link.partlyUnmeasured = true;
      if ((value == null || uncertain.has(link)) && item.note) {
        let seen = notes.get(link);
        if (!seen) { seen = new Set(); notes.set(link, seen); }
        if (!seen.has(item.note)) { seen.add(item.note); link.notes.push(item.note); }
      }
    }
  }
  for (const node of byId.values()) {
    node.unmeasured = node.present && !measured.has(node);
    if (!measured.has(node) || node.valueUnknown) node.exact = null;
    node.children.sort(order);
  }
  if (authoritativeTotal) {
    top.present = true;
    top.valueUnknown = authoritativeTotal.value == null || authoritativeTotal.unknownValue === true;
    top.value = authoritativeTotal.value ?? top.value;
    top.exact = top.valueUnknown ? null : authoritativeTotal.exact ?? null;
    top.unmeasured = false;
    top.final = authoritativeTotal.final ?? top.final;
    top.finalUnknown = authoritativeTotal.finalUnknown === true || authoritativeTotal.final === null;
    if (authoritativeTotal.note && !top.notes.includes(authoritativeTotal.note)) top.notes.push(authoritativeTotal.note);
  }
  return top;
}

/** Folds each node's drawable children beyond `limit` (the palette's colours) into
 * one "Other" slice holding the smallest by their whole amount, amounts not yet
 * present included; the folded slices are its children, a ring further out, so
 * zooming into it shows them, folded again for that view only. Whole amounts do
 * not change during playback, so the same slices fold throughout. */
export function foldTree(node: SunburstNode, limit = CATEGORY_LIMIT, label = 'Other'): SunburstNode {
  foldChildren(node, limit, label);
  deepen(node, node.depth);
  return node;
}

function foldChildren(node: SunburstNode, limit: number, label: string): void {
  for (const child of node.children) if (!child.other) foldChildren(child, limit, label);
  const shown = node.children.filter(drawable);
  if (shown.length > limit) {
    const folded = shown.slice(limit - 1);
    const surplus = new Set(folded);
    const other: SunburstNode = { id: `${node.id}${SEPARATOR}\u0001other`, key: '\u0001other', label, depth: node.depth + 1, parent: node,
      children: folded, value: 0, exact: 0n, valueUnknown: false, final: 0, finalUnknown: false, present: false, unmeasured: false, partlyUnmeasured: false, notes: [], unknown: {}, counted: new Set(), count: 0, other: true };
    const notes = new Set<string>();
    for (const child of folded) {
      other.value += child.value; other.final += child.final; add(other.unknown, child.unknown);
      other.valueUnknown ||= child.valueUnknown;
      other.finalUnknown ||= child.finalUnknown;
      for (const id of child.counted) other.counted.add(id);
      other.count = other.counted.size;
      other.present ||= child.present;
      other.partlyUnmeasured ||= child.partlyUnmeasured;
      for (const note of child.notes) if (!notes.has(note)) { notes.add(note); other.notes.push(note); }
      child.parent = other;
    }
    // Other is what its present children are: measured if any is (its exact amount
    // the sum of theirs, when each has one), else not measured.
    const measured = folded.filter(child => child.present && !child.unmeasured);
    other.unmeasured = other.present && measured.length === 0;
    other.exact = measured.length && !other.valueUnknown ? measured.reduce<bigint | null>((sum, child) => sum != null && child.exact != null ? sum + child.exact : null, 0n) : null;
    node.children = [...node.children.filter(child => !surplus.has(child)), other].sort(order);
  }
}

function deepen(node: SunburstNode, depth: number): void {
  node.depth = depth;
  for (const child of node.children) deepen(child, depth + 1);
}

/** Every node of a tree, parents first. */
export function walk(node: SunburstNode, into: SunburstNode[] = []): SunburstNode[] {
  into.push(node);
  for (const child of node.children) walk(child, into);
  return into;
}

/** A node's ancestors from the root, itself last. */
export function pathTo(node: SunburstNode): SunburstNode[] {
  const path: SunburstNode[] = [];
  for (let at: SunburstNode | null = node; at; at = at.parent) path.unshift(at);
  return path;
}

/** The node with `id`, else its nearest ancestor still in the tree (ids are keys
 * from the root, and a folded slice keeps its own), else the root: a zoom survives
 * the tree changing under it. */
export function findNode(root: SunburstNode, id: string | null): SunburstNode {
  if (!id) return root;
  const byId = new Map(walk(root).map(node => [node.id, node]));
  for (let at = id; ; at = at.slice(0, at.lastIndexOf(SEPARATOR))) {
    const found = byId.get(at);
    if (found) return found;
    if (!at.includes(SEPARATOR)) return root;
  }
}

/** Reopens the zoom's path on a fresh tree: only Other nodes on that path fold
 * their parts again. Keeping the path preserves their lazy ancestors on playback. */
export function foldView(root: SunburstNode, path: readonly string[]): SunburstNode {
  let focus = root;
  for (const id of path) {
    focus = findNode(root, id);
    if (focus.other) foldTree(focus);
  }
  return focus;
}

export type Growth = 'share' | 'fill';
const TAU = Math.PI * 2;

/** Where a slice goes: its angles clockwise from the top, in radians, and its ring
 * counted from the zoomed slice (1 is the innermost ring shown). */
export interface ArcTarget { a0: number; a1: number; ring: number; shown: boolean }

/** How far round the circle the zoomed slice's children reach: all the way (Share),
 * or as far as its amount has come toward its whole amount (Fill). */
export function sweepOf(focus: SunburstNode, growth: Growth): number {
  if (focus.valueUnknown) return 0;
  if (focus.finalUnknown) return TAU;
  if (growth === 'share') return TAU;
  return focus.final > 0 ? TAU * Math.max(0, Math.min(1, focus.value / focus.final)) : focus.value > 0 ? TAU : 0;
}

/** Every node's place with the view zoomed into `focus`: the tree is partitioned by
 * amount, then mapped so the focus fills the sweep. `rings` rings show; nodes
 * outside the focus close to nothing at its edges. */
export function arcTargets(root: SunburstNode, focus: SunburstNode, growth: Growth, rings = 2): Map<string, ArcTarget> {
  const opened = new Set(pathTo(focus));
  const spans = new Map<string, [number, number, boolean]>();
  const part = (node: SunburstNode, a0: number, a1: number, closed = false) => {
    const blocked = closed || (node.valueUnknown && !(opened.has(node) && node !== focus));
    spans.set(node.id, [a0, a1, blocked]);
    let at = a0;
    for (const child of node.children) {
      const width = node.value > 0 ? (a1 - a0) * child.value / node.value : 0;
      part(child, at, at + width, blocked || (node.other === true && !opened.has(node)));
      at += width;
    }
  };
  part(root, 0, TAU);
  const [f0, f1] = spans.get(focus.id) ?? [0, TAU];
  const sweep = sweepOf(focus, growth);
  const span = f1 - f0;
  const map = (angle: number) => span > 0 ? Math.max(0, Math.min(sweep, (angle - f0) / span * sweep)) : 0;
  const targets = new Map<string, ArcTarget>();
  for (const node of walk(root)) {
    const [a0, a1, closed] = spans.get(node.id)!;
    const ring = node.depth - focus.depth;
    const t0 = map(a0);
    const t1 = map(a1);
    targets.set(node.id, { a0: t0, a1: t1, ring, shown: !closed && ring >= 1 && ring <= rings && node.value > 0 && t1 - t0 > 1e-9 });
  }
  return targets;
}

/** Colours with the view zoomed into `focus`: the innermost ring's drawable slices
 * take the palette in their stable order (so a slice keeps its colour while playback
 * grows the others), and each ring outward shades its parent's colour, alternately
 * toward the text and the surface colour. Parts that are never drawn (not measured,
 * or with nothing but parts not counted) take no slot and no colour, so they cannot
 * push a slice onto a reused colour. Nodes not under the focus keep no colour. */
export function sliceColours(focus: SunburstNode, palette: readonly string[], shades: { light: string; dark: string }): Map<string, string> {
  const colours = new Map<string, string>();
  const paint = (node: SunburstNode, colour: string) => {
    colours.set(node.id, colour);
    if (node.other) return;
    node.children.filter(drawable).forEach((child, index) => {
      const step = index === 0 ? 0 : 0.12 + 0.08 * Math.floor((index - 1) / 2);
      paint(child, step ? mix(colour, index % 2 ? shades.light : shades.dark, step) : colour);
    });
  };
  focus.children.filter(drawable).forEach((child, index) => paint(child, palette[index % Math.max(1, palette.length)] ?? shades.light));
  return colours;
}

/** One frame of a colour gliding toward another: it moves in unrounded channels and
 * arrives exactly once within half a channel step, so the glide always ends (a
 * rounded colour could stall short of it and keep the chart drawing). */
export function approachColour(current: Rgb, target: Rgb, amount: number): Rgb {
  if (!(amount < 1)) return target;
  const t = Math.max(0, amount);
  const next = { r: current.r + (target.r - current.r) * t, g: current.g + (target.g - current.g) * t, b: current.b + (target.b - current.b) * t };
  return Math.max(Math.abs(next.r - target.r), Math.abs(next.g - target.g), Math.abs(next.b - target.b)) < 0.5 ? target : next;
}

/** A slice as drawn: its angles and radii this frame. */
export interface DrawnSlice { id: string; a0: number; a1: number; inner: number; outer: number; alpha: number }

/** The disc's size: the centre's radius and each ring's width fit the smaller side. */
export interface SunGeometry { cx: number; cy: number; radius: number; hole: number; ring: number }
export function sunGeometry(width: number, height: number, rings = 2, pad = 10): SunGeometry {
  const radius = Math.max(8, Math.min(width, height) / 2 - pad);
  const hole = radius * 0.36;
  return { cx: width / 2, cy: height / 2, radius, hole, ring: (radius - hole) / Math.max(1, rings) };
}

/** The angle clockwise from the top at a point, in [0, 2π). */
export function angleAt(g: SunGeometry, x: number, y: number): number {
  const angle = Math.atan2(y - g.cy, x - g.cx) + Math.PI / 2;
  return angle < 0 ? angle + TAU : angle;
}

/** What is under a point: the centre (to zoom out), a drawn slice, or nothing. Only
 * slices drawn at least half visible count, so a fading slice is not hit. */
export function sliceAt(slices: readonly DrawnSlice[], g: SunGeometry, x: number, y: number): string | 'centre' | null {
  const distance = Math.hypot(x - g.cx, y - g.cy);
  if (distance < g.hole - 4) return 'centre';
  const angle = angleAt(g, x, y);
  for (const slice of slices) {
    if (slice.alpha < 0.5) continue;
    if (distance >= slice.inner && distance < slice.outer && angle >= slice.a0 && angle < slice.a1) return slice.id;
  }
  return null;
}

/** Whether a label fits inside its slice, written straight across the ring's
 * middle (tangent to it): the text's box, `textWidth` wide and `fontSize` tall with
 * padding, must keep its outer corners inside the outer radius, its inner edge
 * outside the inner radius, and its corners within the slice's angles. A straight
 * line of text reaches further out at its ends than at its middle, so a long label
 * on a wide slice can cross the outer edge though the arc is long enough. */
export function labelFits(slice: Pick<DrawnSlice, 'a0' | 'a1' | 'inner' | 'outer'>, textWidth: number, fontSize: number): boolean {
  const pad = 3;
  const middle = (slice.inner + slice.outer) / 2;
  const half = textWidth / 2 + pad;
  const top = middle + fontSize / 2 + pad;
  const bottom = middle - fontSize / 2 - pad;
  if (bottom <= slice.inner || Math.hypot(top, half) > slice.outer) return false;
  // The corners' angle either side of the middle, seen from the centre.
  return 2 * Math.atan2(half, bottom) <= slice.a1 - slice.a0;
}

/** The slices the keys step through, ring by ring and clockwise in each. */
export function keyRings(root: SunburstNode, targets: ReadonlyMap<string, ArcTarget>): SunburstNode[][] {
  const rings: SunburstNode[][] = [];
  for (const node of walk(root)) {
    const target = targets.get(node.id);
    if (!target?.shown) continue;
    (rings[target.ring - 1] ??= []).push(node);
  }
  return rings.filter(Boolean).map(ring => ring.sort((a, b) => targets.get(a.id)!.a0 - targets.get(b.id)!.a0));
}

export type SunKey = 'ArrowLeft' | 'ArrowRight' | 'ArrowUp' | 'ArrowDown' | 'Home' | 'End';

/** The slice a key moves the keyboard to: Left and Right go round its ring (wrapping,
 * as a circle does), Down goes out to its first child shown, Up back in to its
 * parent, Home and End to either end of its ring. The first press chooses the first
 * slice of the innermost ring; a slice no longer shown starts over there. */
export function keyedSlice(key: SunKey, current: string | null, rings: readonly (readonly SunburstNode[])[]): SunburstNode | null {
  const first = rings[0]?.[0] ?? null;
  const ringIndex = current == null ? -1 : rings.findIndex(ring => ring.some(node => node.id === current));
  if (ringIndex < 0) return first;
  const ring = rings[ringIndex];
  const index = ring.findIndex(node => node.id === current);
  const node = ring[index];
  switch (key) {
    case 'ArrowRight': return ring[(index + 1) % ring.length];
    case 'ArrowLeft': return ring[(index - 1 + ring.length) % ring.length];
    case 'Home': return ring[0];
    case 'End': return ring[ring.length - 1];
    case 'ArrowDown': return rings[ringIndex + 1]?.find(child => child.parent === node) ?? node;
    case 'ArrowUp': return ringIndex > 0 && node.parent ? rings[ringIndex - 1].find(parent => parent === node.parent) ?? node : node;
  }
}

/** A share as the legend and tooltip write it: to one decimal, "<0.1%" for a sliver. */
export function shareText(part: number, whole: number): string {
  if (!(whole > 0)) return '—';
  const share = part / whole * 100;
  if (share > 0 && share < 0.1) return '<0.1%';
  return `${Math.round(share * 10) / 10}%`;
}

/** How a widget words its sunburst: exact amounts, their unit, what the amount is,
 * its reasons for parts not counted, and any rows of its own for a slice. */
export interface SunburstText {
  /** An amount as written: `exact` when the tree has it (never rounded through a
   * number), else `value`. */
  format: (value: number, exact: bigint | null) => string;
  unit: string;
  /** The tooltip's name for the amount, such as "Spent". */
  valueLabel: string;
  /** A reason for parts not counted, as a row label. */
  unknownLabel?: (reason: string) => string;
  /** The name of the items' `count` (such as "Projects"); none leaves it out. */
  countLabel?: string;
  /** Rows of the widget's own for a slice. */
  describe?: (node: SunburstNode) => readonly TooltipRow[];
}

/** How many parts of a slice are not counted, whatever the reason. */
export const unknownTotal = (node: SunburstNode) => Object.values(node.unknown).reduce((sum, count) => sum + count, 0);

/** A slice's tooltip, fullest first: its path, exact amount, its share of its parent
 * and of the slice zoomed into, the widget's rows, then what is not counted by
 * reason (the compact form counts them in one row) and why parts are not measured. */
export function sliceTooltip(node: SunburstNode, focus: SunburstNode, text: SunburstText): TooltipContent[] {
  const path = pathTo(node).slice(1).map(step => step.label).join(' › ') || node.label;
  const unmeasured = notMeasured(node);
  const rows: TooltipRow[] = [node.valueUnknown
    ? { label: text.valueLabel, value: 'Not known', tone: 'muted' }
    : unmeasured
    ? { label: text.valueLabel, value: 'Not measured', tone: 'muted' }
    : { label: text.valueLabel, value: `${amountText(node, text)} ${text.unit}`, strong: true }];
  if (!node.valueUnknown && !unmeasured && node.parent && !node.parent.valueUnknown) rows.push({ label: `Of ${node.parent.label}`, value: shareText(node.value, node.parent.value) });
  if (!node.valueUnknown && !unmeasured && node.parent && node.parent !== focus && node !== focus && !focus.valueUnknown) rows.push({ label: `Of ${focus.label}`, value: shareText(node.value, focus.value) });
  if (text.countLabel && node.count > 0) rows.push({ label: text.countLabel, value: String(node.count) });
  rows.push(...(text.describe?.(node) ?? []));
  const total = unknownTotal(node);
  const reasons = Object.entries(node.unknown).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  const counted: TooltipRow = { label: 'Not counted', value: String(total), divider: true, tone: 'muted' };
  const full: TooltipRow[] = total ? [counted, ...reasons.map(([reason, count]): TooltipRow =>
    ({ label: text.unknownLabel?.(reason) ?? reason, value: String(count), indent: true, tone: 'muted' }))] : [];
  const note = node.notes.length ? node.notes.join(' ') : undefined;
  const base = { title: path, icon: node.icon, note };
  return [{ ...base, rows: [...rows, ...full] }, { ...base, rows: total ? [...rows, counted] : rows }];
}

/** A node's amount as written, exactly when the tree has it. */
export const amountText = (node: SunburstNode, text: SunburstText) => node.valueUnknown ? 'Not known' : text.format(node.value, node.exact);

/** How a node is read out: "not measured", else its amount (and share of `whole`),
 * and what is not counted. */
export function spokenAmount(node: SunburstNode, whole: SunburstNode | null, text: SunburstText): string {
  if (node.valueUnknown) return `not known${node.notes.length ? `: ${node.notes.join(' ')}` : ''}`;
  if (notMeasured(node)) return 'not measured';
  const unknown = unknownTotal(node);
  if (!(node.value > 0) && unknown) return `nothing counted, ${unknown} not counted`;
  const amount = `${amountText(node, text)} ${text.unit}${whole && !whole.valueUnknown ? `, ${shareText(node.value, whole.value)}` : ''}`;
  return unknown ? `${amount}, ${unknown} not counted` : amount;
}

/** The zoomed slice read out: its amount and each listed part's, for assistive
 * technology, saying "not measured" as the centre does. */
export function sunburstSummary(focus: SunburstNode, text: SunburstText): string {
  const parts = focus.children.filter(listed).map(child => `${child.label}: ${spokenAmount(child, focus, text)}`);
  return [`${focus.label}: ${spokenAmount(focus, null, text)}`, ...parts].join('; ');
}
