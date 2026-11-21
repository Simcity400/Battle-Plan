import { mix } from '../../shell/appearance/color.ts';
import type { Rgb } from '../../shell/appearance/color';
import { CATEGORY_LIMIT } from '../palette.ts';
import type { TooltipContent, TooltipRow } from '../tooltip';
import { fractionSum } from '../valueAxis.ts';


export interface SunburstKey {
  id: string; label: string; icon?: string | null;
  countryFlag?: TooltipRow['countryFlag'];
  colour?: string;
  last?: boolean;
  keepZero?: boolean;
  stretch?: number | string | null;
}

export interface SunburstItem {
  path: readonly SunburstKey[];
  value: number | null;
  exact?: bigint | null;
  exactDenominator?: bigint;
  unknownValue?: boolean;
  unknownLevels?: readonly boolean[];
  localUnknownLevels?: readonly boolean[];
  present?: boolean;
  final?: number | null;
  weight?: number;
  finalUnknown?: boolean;
  finalUnknownLevels?: readonly boolean[];
  localFinalUnknownLevels?: readonly boolean[];
  unknown?: Readonly<Record<string, number>>;
  note?: string;
  counted?: readonly string[];
  count?: number;
}

export type SunburstTotal = Pick<SunburstItem, 'value' | 'exact' | 'exactDenominator' | 'unknownValue' | 'final' | 'finalUnknown' | 'note'>;

export interface SunburstNode {
  id: string;
  key: string;
  label: string;
  icon?: string;
  countryFlag?: TooltipRow['countryFlag'];
  colour?: string;
  last?: boolean;
  keepZero?: boolean;
  stretch?: number | string | null;
  depth: number;
  parent: SunburstNode | null;
  children: SunburstNode[];
  value: number;
  exact: bigint | null;
  exactDenominator?: bigint;
  valueUnknown: boolean;
  final: number;
  weight?: number;
  finalUnknown: boolean;
  corrections: boolean;
  present: boolean;
  unmeasured: boolean;
  partlyUnmeasured: boolean;
  notes: string[];
  unknown: Record<string, number>;
  counted: Set<string>;
  aggregateCount: number;
  count: number;
  other?: boolean;
}

const SEPARATOR = '\u0000';
const MAX_SAFE_GEOMETRY = BigInt(Number.MAX_SAFE_INTEGER);
const add = (into: Record<string, number>, from: Readonly<Record<string, number>> | undefined) => {
  for (const [reason, count] of Object.entries(from ?? {})) if (count > 0) into[reason] = (into[reason] ?? 0) + count;
};
const hasAmount = (node: SunburstNode) => node.value !== 0 || node.exact != null && node.exact !== 0n;

export const drawable = (node: SunburstNode) => (node.weight ?? node.final) !== 0 || hasAmount(node);

export const listed = (node: SunburstNode): boolean => node.present && (node.valueUnknown || node.unmeasured || node.partlyUnmeasured || hasAmount(node) || node.corrections || node.keepZero || unknownTotal(node) > 0 || node.children.some(listed));

export const hasCorrections = (node: SunburstNode) => node.corrections;

export const notMeasured = (node: SunburstNode) => node.unmeasured || (node.partlyUnmeasured && !hasAmount(node));

function order(a: SunburstNode, b: SunburstNode): number {
  return Number(Boolean(a.last || a.other)) - Number(Boolean(b.last || b.other))
    || (b.weight ?? b.final) - (a.weight ?? a.final) || (a.key < b.key ? -1 : a.key > b.key ? 1 : 0);
}

export function groupTree(items: readonly SunburstItem[], root: { id: string; label: string }, shown: readonly boolean[] = [], authoritativeTotal?: SunburstTotal): SunburstNode {
  const top: SunburstNode = { id: root.id, key: root.id, label: root.label, depth: 0, parent: null, children: [], value: 0, exact: 0n, valueUnknown: false, final: 0, finalUnknown: false, corrections: false, present: false, unmeasured: true, partlyUnmeasured: false, notes: [], unknown: {}, counted: new Set(), aggregateCount: 0, count: 0 };
  const byId = new Map<string, SunburstNode>([[top.id, top]]);
  const measured = new Set<SunburstNode>();
  const notes = new Map<SunburstNode, Set<string>>();
  for (const item of items) {
    let node = top;
    const chain = [top];
    const uncertain = new Set<SunburstNode>();
    const finalUncertain = new Set<SunburstNode>();
    for (const [level, key] of item.path.entries()) {
      const visible = shown[level] ?? true;
      if (visible) {
        const id = `${node.id}${SEPARATOR}${key.id}`;
        let child = byId.get(id);
        if (!child) {
          child = { id, key: key.id, label: key.label, icon: key.icon ?? undefined, countryFlag: key.countryFlag, colour: key.colour, last: key.last, keepZero: key.keepZero,
            stretch: key.stretch,
            depth: node.depth + 1, parent: node, children: [], value: 0, exact: 0n, valueUnknown: false, final: 0, finalUnknown: false, corrections: false, present: false, unmeasured: true, partlyUnmeasured: false, notes: [], unknown: {}, counted: new Set(), aggregateCount: 0, count: 0 };
          byId.set(id, child);
          node.children.push(child);
        }
        node = child;
        chain.push(child);
      }
      if (item.unknownLevels?.[level] && item.present !== false) for (const link of chain) uncertain.add(link);
      if (item.finalUnknownLevels?.[level]) for (const link of chain) finalUncertain.add(link);
      if (!visible) continue;
      if (item.localUnknownLevels?.[level] && item.present !== false) uncertain.add(node);
      if (item.localFinalUnknownLevels?.[level]) finalUncertain.add(node);
    }
    const present = item.present !== false;
    const value = present ? item.value : null;
    const final = item.final === undefined ? item.value : item.final;
    const correction = value != null && (value < 0 || item.exact != null && item.exact < 0n)
      || final != null && final < 0;
    for (const link of chain) {
      if (item.weight !== undefined || link.weight !== undefined)
        link.weight = (link.weight ?? link.final) + (item.weight ?? final ?? 0);
      if (final != null) link.final += final;
      link.finalUnknown ||= item.finalUnknown === true || finalUncertain.has(link);
      link.corrections ||= correction;
      if (!present) continue;
      link.present = true;
      link.valueUnknown ||= item.unknownValue === true || uncertain.has(link);
      if (value != null) {
        link.value += value;
        addExact(link, item.exact, item.exactDenominator);
        measured.add(link);
      }
      add(link.unknown, item.unknown);
      for (const id of item.counted ?? []) link.counted.add(id);
      link.aggregateCount += item.count ?? 0;
      link.count = link.counted.size + link.aggregateCount;
      if (value == null) link.partlyUnmeasured = true;
      if (item.note) {
        let seen = notes.get(link);
        if (!seen) { seen = new Set(); notes.set(link, seen); }
        if (!seen.has(item.note)) { seen.add(item.note); link.notes.push(item.note); }
      }
    }
  }
  for (const node of byId.values()) {
    node.unmeasured = node.present && !measured.has(node);
    if (!measured.has(node) || node.valueUnknown) node.exact = null;
    recoverGeometry(node);
    node.children.sort(order);
  }
  if (authoritativeTotal) {
    top.present = true;
    top.valueUnknown = authoritativeTotal.value == null || authoritativeTotal.unknownValue === true;
    top.value = authoritativeTotal.value ?? top.value;
    top.exact = top.valueUnknown ? null : authoritativeTotal.exact ?? null;
    top.exactDenominator = authoritativeTotal.exactDenominator;
    top.unmeasured = false;
    top.final = authoritativeTotal.final ?? top.final;
    top.finalUnknown = authoritativeTotal.finalUnknown === true || authoritativeTotal.final === null;
    top.corrections ||= top.value < 0 || top.final < 0 || top.exact != null && top.exact < 0n;
    recoverGeometry(top);
    if (authoritativeTotal.note && !top.notes.includes(authoritativeTotal.note)) top.notes.push(authoritativeTotal.note);
  }
  return top;
}

export function sunburstTree(items: readonly SunburstItem[], root: { id: string; label: string },
  shown?: readonly boolean[], options: { fold?: boolean; unavailableReason?: string | null; authoritativeTotal?: SunburstTotal } = {}): SunburstNode {
  if (options.unavailableReason != null) return groupTree([{ path: [], value: null, note: options.unavailableReason }], root);
  const tree = groupTree(items, root, shown, options.authoritativeTotal);
  return options.fold === false ? tree : foldTree(tree);
}

export function foldTree(node: SunburstNode, limit: number = CATEGORY_LIMIT, label = 'Other'): SunburstNode {
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
      children: folded, value: 0, exact: 0n, valueUnknown: false, final: 0, finalUnknown: false, corrections: false, present: false, unmeasured: false, partlyUnmeasured: false, notes: [], unknown: {}, counted: new Set(), aggregateCount: 0, count: 0, other: true };
    const notes = new Set<string>();
    for (const child of folded) {
      if (child.weight !== undefined || other.weight !== undefined)
        other.weight = (other.weight ?? other.final) + (child.weight ?? child.final);
      other.value += child.value; other.final += child.final; add(other.unknown, child.unknown);
      other.valueUnknown ||= child.valueUnknown;
      other.finalUnknown ||= child.finalUnknown;
      other.corrections ||= child.corrections;
      for (const id of child.counted) other.counted.add(id);
      other.aggregateCount += child.aggregateCount;
      other.count = other.counted.size + other.aggregateCount;
      other.present ||= child.present;
      other.partlyUnmeasured ||= child.partlyUnmeasured;
      for (const note of child.notes) if (!notes.has(note)) { notes.add(note); other.notes.push(note); }
      child.parent = other;
    }
    const measured = folded.filter(child => child.present && !child.unmeasured);
    other.unmeasured = other.present && measured.length === 0;
    if (measured.length && !other.valueUnknown) for (const child of measured) addExact(other, child.exact, child.exactDenominator);
    else other.exact = null;
    recoverGeometry(other);
    node.children = [...node.children.filter(child => !surplus.has(child)), other].sort(order);
  }
}

function addExact(node: SunburstNode, amount: bigint | null | undefined, denominator = 1n): void {
  if (node.exact == null || amount == null) { node.exact = null; return; }
  if (denominator === 1n && node.exactDenominator == null) { node.exact += amount; return; }
  const [sum, units] = fractionSum([[node.exact, node.exactDenominator ?? 1n], [amount, denominator]]);
  node.exact = sum;
  node.exactDenominator = units === 1n ? undefined : units;
}

function recoverGeometry(node: SunburstNode): void {
  if (node.value !== 0 || node.valueUnknown || node.exact == null) return;
  const denominator = node.exactDenominator ?? 1n;
  if (node.exact >= -MAX_SAFE_GEOMETRY && node.exact <= MAX_SAFE_GEOMETRY && denominator > 0n && denominator <= MAX_SAFE_GEOMETRY)
    node.value = Number(node.exact) / Number(denominator);
}

function deepen(node: SunburstNode, depth: number): void {
  node.depth = depth;
  for (const child of node.children) deepen(child, depth + 1);
}

export function walk(node: SunburstNode, into: SunburstNode[] = []): SunburstNode[] {
  into.push(node);
  for (const child of node.children) walk(child, into);
  return into;
}

export function pathTo(node: SunburstNode): SunburstNode[] {
  const path: SunburstNode[] = [];
  for (let at: SunburstNode | null = node; at; at = at.parent) path.unshift(at);
  return path;
}

export function findNode(root: SunburstNode, id: string | null): SunburstNode {
  if (!id) return root;
  const byId = new Map(walk(root).map(node => [node.id, node]));
  for (let at = id; ; at = at.slice(0, at.lastIndexOf(SEPARATOR))) {
    const found = byId.get(at);
    if (found) return found;
    if (!at.includes(SEPARATOR)) return root;
  }
}

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

export interface ArcTarget { a0: number; a1: number; ring: number; shown: boolean }

export function sweepOf(focus: SunburstNode, growth: Growth): number {
  if (focus.valueUnknown) return 0;
  if (focus.finalUnknown) return TAU;
  if (hasCorrections(focus)) return TAU;
  if (growth === 'share') return TAU;
  return focus.final > 0 ? TAU * Math.max(0, Math.min(1, focus.value / focus.final)) : focus.value > 0 ? TAU : 0;
}

export function arcTargets(root: SunburstNode, focus: SunburstNode, growth: Growth, rings = 2): Map<string, ArcTarget> {
  const opened = new Set(pathTo(focus));
  const spans = new Map<string, [number, number, boolean, boolean]>();
  const part = (node: SunburstNode, a0: number, a1: number, closed = false, inFocus = node === focus) => {
    const blocked = closed || (node.valueUnknown && !(opened.has(node) && node !== focus));
    spans.set(node.id, [a0, a1, blocked, inFocus]);
    let at = a0;
    const amount = Math.max(node.value, node.children.reduce((sum, child) => sum + Math.max(0, child.value), 0));
    for (const child of node.children) {
      const width = amount > 0 ? (a1 - a0) * Math.max(0, child.value) / amount : 0;
      part(child, at, at + width, blocked || (node.other === true && !opened.has(node)), inFocus || child === focus);
      at += width;
    }
  };
  part(root, 0, TAU);
  const focusSpan = spans.get(focus.id);
  if (focusSpan && focusSpan[0] === focusSpan[1]) part(focus, 0, TAU);
  const [f0, f1] = spans.get(focus.id) ?? [0, TAU];
  const sweep = sweepOf(focus, growth);
  const span = f1 - f0;
  const map = (angle: number) => span > 0 ? Math.max(0, Math.min(sweep, (angle - f0) / span * sweep)) : 0;
  const targets = new Map<string, ArcTarget>();
  for (const node of walk(root)) {
    const [a0, a1, closed, inFocus] = spans.get(node.id)!;
    const ring = node.depth - focus.depth;
    const t0 = map(a0);
    const t1 = inFocus || opened.has(node) ? map(a1) : t0;
    targets.set(node.id, { a0: t0, a1: t1, ring, shown: inFocus && !closed && ring >= 1 && ring <= rings && node.value > 0 && t1 - t0 > 1e-9 });
  }
  return targets;
}

export function sliceColours(focus: SunburstNode, palette: readonly string[], shades: { light: string; dark: string }): Map<string, string> {
  const colours = new Map<string, string>();
  const paint = (node: SunburstNode, colour: string) => {
    colours.set(node.id, colour);
    if (node.other) return;
    node.children.filter(drawable).forEach((child, index) => {
      const step = index === 0 ? 0 : 0.12 + 0.08 * Math.floor((index - 1) / 2);
      paint(child, child.colour ?? (step ? mix(colour, index % 2 ? shades.light : shades.dark, step) : colour));
    });
  };
  focus.children.filter(child => drawable(child) || child.keepZero && child.colour != null)
    .forEach((child, index) => paint(child, child.colour ?? palette[index % Math.max(1, palette.length)] ?? shades.light));
  return colours;
}

export function approachColour(current: Rgb, target: Rgb, amount: number): Rgb {
  if (!(amount < 1)) return target;
  const t = Math.max(0, amount);
  const next = { r: current.r + (target.r - current.r) * t, g: current.g + (target.g - current.g) * t, b: current.b + (target.b - current.b) * t };
  return Math.max(Math.abs(next.r - target.r), Math.abs(next.g - target.g), Math.abs(next.b - target.b)) < 0.5 ? target : next;
}

export interface DrawnSlice { id: string; a0: number; a1: number; inner: number; outer: number; alpha: number }

export interface SunGeometry { cx: number; cy: number; radius: number; hole: number; ring: number }
export function sunGeometry(width: number, height: number, rings = 2, pad = 10): SunGeometry {
  const radius = Math.max(8, Math.min(width, height) / 2 - pad);
  const hole = radius * 0.36;
  return { cx: width / 2, cy: height / 2, radius, hole, ring: (radius - hole) / Math.max(1, rings) };
}

export function angleAt(g: SunGeometry, x: number, y: number): number {
  const angle = Math.atan2(y - g.cy, x - g.cx) + Math.PI / 2;
  return angle < 0 ? angle + TAU : angle;
}

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

export function labelFits(slice: Pick<DrawnSlice, 'a0' | 'a1' | 'inner' | 'outer'>, textWidth: number, fontSize: number): boolean {
  const pad = 3;
  const middle = (slice.inner + slice.outer) / 2;
  const half = textWidth / 2 + pad;
  const top = middle + fontSize / 2 + pad;
  const bottom = middle - fontSize / 2 - pad;
  if (bottom <= slice.inner || Math.hypot(top, half) > slice.outer) return false;
  return 2 * Math.atan2(half, bottom) <= slice.a1 - slice.a0;
}

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

export function shareText(part: number, whole: number): string {
  if (!(whole > 0)) return '—';
  const share = part / whole * 100;
  if (share > 0 && share < 0.1) return '<0.1%';
  return `${Math.round(share * 10) / 10}%`;
}

export interface SunburstText {
  format: (value: number, exact: bigint | null, denominator?: bigint) => string;
  unit: string;
  valueLabel: string;
  shares?: boolean;
  missingLabel?: string;
  emptyLabel?: string;
  countLabel?: string;
  describe?: (node: SunburstNode) => readonly TooltipRow[];
}

export const unknownTotal = (node: SunburstNode) => Object.values(node.unknown).reduce((sum, count) => sum + count, 0);

export function sliceTooltip(node: SunburstNode, focus: SunburstNode, text: SunburstText): TooltipContent[] {
  const path = pathTo(node).slice(1).map(step => step.label).join(' › ') || node.label;
  const unmeasured = notMeasured(node);
  const rows: TooltipRow[] = [node.valueUnknown
    ? { label: text.valueLabel, value: 'Not known', tone: 'muted' }
    : unmeasured
    ? { label: text.valueLabel, value: text.missingLabel ?? 'Not measured', tone: 'muted' }
    : { label: text.valueLabel, value: `${amountText(node, text)} ${text.unit}${unknownTotal(node) ? ' · partial' : ''}`, strong: true }];
  const shareable = text.shares !== false && (node.value > 0 || node.value === 0 && node.keepZero);
  if (!node.valueUnknown && !unmeasured && shareable && node.parent && !node.parent.valueUnknown && !hasCorrections(node.parent)) rows.push({ label: `Of ${node.parent.label}`, value: shareText(node.value, node.parent.value) });
  if (!node.valueUnknown && !unmeasured && shareable && node.parent && node.parent !== focus && node !== focus && !focus.valueUnknown && !hasCorrections(focus)) rows.push({ label: `Of ${focus.label}`, value: shareText(node.value, focus.value) });
  if (text.countLabel && node.count > 0) rows.push({ label: text.countLabel, value: String(node.count) });
  rows.push(...(text.describe?.(node) ?? []));
  const note = node.notes.length ? node.notes.join(' ') : undefined;
  const base = { title: path, icon: node.icon, countryFlag: node.countryFlag, note };
  return [{ ...base, rows }, { ...base, rows }];
}

export const amountText = (node: SunburstNode, text: SunburstText) => node.valueUnknown ? 'Not known' : text.format(node.value, node.exact, node.exactDenominator);

export function spokenAmount(node: SunburstNode, whole: SunburstNode | null, text: SunburstText): string {
  if (node.valueUnknown) return `not known${node.notes.length ? `: ${node.notes.join(' ')}` : ''}`;
  if (notMeasured(node)) return (text.missingLabel ?? 'Not measured').toLowerCase();
  const unknown = unknownTotal(node);
  const shareable = text.shares !== false && (node.value > 0 || node.value === 0 && node.keepZero);
  const amount = `${amountText(node, text)} ${text.unit}${shareable && whole && !whole.valueUnknown && !hasCorrections(whole) ? `, ${shareText(node.value, whole.value)}` : ''}`;
  return unknown ? `${amount}, partial` : amount;
}

export function sunburstSummary(focus: SunburstNode, text: SunburstText, notes = false): string {
  const parts = focus.children.filter(listed).map(child => `${child.label}: ${spokenAmount(child, focus, text)}`);
  return [`${focus.label}: ${spokenAmount(focus, null, text)}`, ...parts,
    ...(notes && parts.length === 0 && notMeasured(focus) ? focus.notes : [])].join('; ');
}
