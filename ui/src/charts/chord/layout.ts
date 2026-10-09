/** Directed weighted chords. Values here are geometry only; widgets format exact data. */
import type { CountryFlagLook } from '../../components/CountryFlag';
export interface ChordNode { id: string; label: string; color: string; flag?: string | null; countryFlag?: CountryFlagLook }
export interface ChordFlow { id: string; source: string; target: string; value: number }
export interface ChordData { nodes: readonly ChordNode[]; flows: readonly ChordFlow[] }
export interface ChordArc { node: ChordNode; start: number; end: number; incoming: number; outgoing: number; presence: number }
export interface ChordRibbon { flow: ChordFlow; sourceStart: number; sourceEnd: number; targetStart: number; targetEnd: number }
export interface ChordLayout { arcs: ChordArc[]; ribbons: ChordRibbon[] }
export interface ChordSurface { x: number; y: number; radius: number }
export type ChordHit = { kind: 'node' | 'flow'; id: string };

const TAU = 2 * Math.PI;
export const ARC_BAND = 7;
export const RIBBON_INSET = 8;
export const ARC_GAP = 0.04;
/** Retain the approved spacing for small diagrams. Any number of visible arcs
 * shares at most half the ring as gaps, leaving positive room for their values. */
export const chordGap = (presence: number) => Math.min(ARC_GAP, Math.PI / Math.max(1, presence));

/** Style-6's placement, bounded so a small widget never gets a negative radius. */
export function chordSurface(width: number, height: number): ChordSurface {
  return { x: width / 2 - 40, y: height / 2 + 4, radius: Math.max(0, Math.min(width - 180, height) / 2 - 26) };
}

/** Linear in nodes + flows. Input order is the order of both arcs and their segments.
 * Zero nodes occupy no angle; presence eases their gap as they enter or leave. */
export function layoutChord(data: ChordData, presence?: ReadonlyMap<string, number>, ringPresence = 1): ChordLayout {
  const arcs = data.nodes.map(node => ({ node, start: 0, end: 0, incoming: 0, outgoing: 0, presence: 0, offset: 0 }));
  const byId = new Map(arcs.map(arc => [arc.node.id, arc]));
  const pairs: { flow: ChordFlow; source: typeof arcs[number]; target: typeof arcs[number] }[] = [];
  let total = 0;
  for (const flow of data.flows) {
    if (flow.value === 0) continue;
    const source = byId.get(flow.source)!, target = byId.get(flow.target)!;
    source.outgoing += flow.value;
    target.incoming += flow.value;
    total += flow.value * 2;
    pairs.push({ flow, source, target });
  }
  let gaps = 0;
  for (const arc of arcs) {
    arc.presence = arc.incoming + arc.outgoing > 0 ? Math.max(0, Math.min(1, presence?.get(arc.node.id) ?? 1)) : 0;
    gaps += arc.presence;
  }
  // Departing and entering nodes share the ring by their eased presence.
  const gap = chordGap(gaps);
  const unit = total > 0 ? (TAU - gap * gaps) * Math.max(0, Math.min(1, ringPresence)) / total : 0;
  let angle = -Math.PI / 2;
  for (const arc of arcs) {
    arc.start = angle;
    arc.end = angle + (arc.incoming + arc.outgoing) * unit;
    arc.offset = angle;
    angle = arc.end + gap * arc.presence;
  }
  const ribbons = pairs.map(({ flow, source, target }): ChordRibbon => {
    const width = flow.value * unit;
    const sourceStart = source.offset;
    source.offset += width;
    // Read after advancing source: a self-flow consumes two distinct segments.
    const targetStart = target.offset;
    target.offset += width;
    return { flow, sourceStart, sourceEnd: sourceStart + width, targetStart, targetEnd: targetStart + width };
  });
  return { arcs, ribbons };
}

export const chordStops = (layout: ChordLayout): ChordHit[] => [
  ...layout.ribbons.filter(ribbon => ribbon.sourceEnd > ribbon.sourceStart && ribbon.targetEnd > ribbon.targetStart).map(ribbon => ({ kind: 'flow' as const, id: ribbon.flow.id })),
  ...layout.arcs.filter(arc => arc.end > arc.start).map(arc => ({ kind: 'node' as const, id: arc.node.id })),
];

export function stepChord(stops: readonly ChordHit[], current: ChordHit | null, delta: number): ChordHit | null {
  if (!stops.length) return null;
  const index = current ? stops.findIndex(stop => stop.kind === current.kind && stop.id === current.id) : -1;
  return stops[index < 0 ? (delta < 0 ? stops.length - 1 : 0) : (index + delta + stops.length) % stops.length];
}

function inAngle(angle: number, start: number, end: number): boolean {
  const offset = ((angle - start) % TAU + TAU) % TAU;
  return offset < end - start;
}

/** Crossings of a rightward ray with a quadratic whose control point is the centre. */
function quadraticCrossings(x: number, y: number, radius: number, start: number, end: number): number {
  const x0 = Math.cos(start) * radius, y0 = Math.sin(start) * radius;
  const x1 = Math.cos(end) * radius, y1 = Math.sin(end) * radius;
  const a = y0 + y1, b = -2 * y0, c = y0 - y;
  const crosses = (t: number) => t >= 0 && t < 1 && Math.abs(2 * a * t + b) > 1e-10
    && x0 * (1 - t) ** 2 + x1 * t ** 2 > x;
  if (Math.abs(a) < 1e-10) return Math.abs(b) > 1e-10 && crosses(-c / b) ? 1 : 0;
  const discriminant = b * b - 4 * a * c;
  if (discriminant <= 0) return 0;
  const root = Math.sqrt(discriminant);
  return Number(crosses((-b - root) / (2 * a))) + Number(crosses((-b + root) / (2 * a)));
}

function arcCrossings(x: number, y: number, radius: number, start: number, end: number): number {
  if (Math.abs(y) >= radius) return 0;
  const angle = Math.asin(y / radius);
  return Number(inAngle(angle, start, end) && radius * Math.cos(angle) > x)
    + Number(inAngle(Math.PI - angle, start, end) && -radius * Math.cos(angle) > x);
}

/** Exact ray intersections with the same circular arcs and quadratics Canvas draws. */
export function ribbonContains(ribbon: ChordRibbon, x: number, y: number, radius: number): boolean {
  if (!(radius > 0) || x * x + y * y > radius * radius) return false;
  const { sourceStart: a, sourceEnd: b, targetStart: c, targetEnd: d } = ribbon;
  const crossings = arcCrossings(x, y, radius, a, b) + quadraticCrossings(x, y, radius, b, c)
    + arcCrossings(x, y, radius, c, d) + quadraticCrossings(x, y, radius, d, a);
  return crossings % 2 === 1;
}

/** Topmost item first, matching paint order. Arcs take precedence over ribbons. */
export function hitChord(layout: ChordLayout, surface: ChordSurface, x: number, y: number): ChordHit | null {
  x -= surface.x; y -= surface.y;
  const distance = Math.hypot(x, y), angle = Math.atan2(y, x);
  if (surface.radius > 0 && distance >= Math.max(0, surface.radius - ARC_BAND) && distance <= surface.radius) {
    for (const arc of layout.arcs) if (inAngle(angle, arc.start, arc.end)) return { kind: 'node', id: arc.node.id };
  }
  for (let index = layout.ribbons.length - 1; index >= 0; index--) {
    const ribbon = layout.ribbons[index];
    if (ribbonContains(ribbon, x, y, surface.radius - RIBBON_INSET)) return { kind: 'flow', id: ribbon.flow.id };
  }
  return null;
}
