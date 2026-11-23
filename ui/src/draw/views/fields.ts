import { liveKey } from '../../charts/live.ts';
import { CellGroup, sectionGroups, sectionOf, sectionValues, valuedGroup, type GroupValues } from '../../dashboard/follow/liveCells.ts';
import type { FollowedPanel } from '../../dashboard/follow/message';
import type { Picture } from '../../dashboard/follow/facts';
import type { LaneView, Painted, Scene, Value } from '../lane.ts';

export type Structure = (panel: FollowedPanel, hour: number) => string | undefined;
export type LiveMap = Map<string, number | bigint | string>;

export interface ModelField { bytes: Uint8Array | null; data?: unknown; refused: string | null; structure?: string }

const groups = new WeakMap<object, GroupValues>();

function groupOf(value: Value, spec: Parameters<typeof valuedGroup>[0]): GroupValues {
  if (value.kind === 'seed') return valuedGroup(spec, value.values);
  let held = groups.get(value);
  if (!held) {
    const group = new CellGroup();
    if (value.kind === 'answer') group.land(value.answer);
    groups.set(value, held = group);
  }
  return held;
}

export function liveValues(picture: Picture<Value>): LiveMap | null {
  const head = picture.parts[0]!.value;
  if (head.kind !== 'panel' || head.refused !== null) return null;
  const section = sectionOf(head.data);
  if (!section) return null;
  const cells = picture.parts[0]!.cells;
  const live = sectionValues(section, sectionGroups(section, id => {
    const at = cells.indexOf(id), part = at < 0 ? undefined : picture.parts[at + 1];
    const spec = section.groups.find(group => group.follow === id);
    return part && spec ? groupOf(part.value, spec) : undefined;
  }));
  const out: LiveMap = new Map();
  for (let i = 0; i < section.cells.length; i += 3 + section.cells[i + 2]! * 3) {
    const key = liveKey(section.rows[section.cells[i]!]!, section.kinds[section.cells[i + 1]!]!.id);
    const raw = live.raw?.(key);
    out.set(key, raw !== undefined && !Number.isSafeInteger(Number(raw)) ? raw : live.get(key) ?? live.missing(key));
  }
  return out;
}


export const sameLive = (a: LiveMap | null, b: LiveMap | null) => {
  if (a === b) return true;
  if (!a || !b || a.size !== b.size) return false;
  for (const [key, value] of a) if (b.get(key) !== value) return false;
  return true;
};

export function fieldsOf(scene: Scene, hour: number, structureOf: (product: string | undefined) => Structure | undefined,
  state: Map<string, unknown>, content: () => string): Painted['fields'] {
  const fields: Record<string, { value: unknown; content: string }> = {};
  scene.subs.forEach((sub, at) => {
    const picture = scene.pictures[at]!, head = picture.parts[0]!;
    if (head.value.kind === 'answer') {
      fields[`answer:${sub.id}`] = { value: head.value.body, content: `answer:${head.version}` };
      return;
    }
    if (head.value.kind !== 'panel') return;
    const value = head.value;
    const panel: FollowedPanel = { data: value.data, refused: value.refused, ...value.bytes ? { bytes: value.bytes } : {} };
    let structure: string | undefined;
    if (value.refused === null) try { structure = structureOf(sub.product)?.(panel, hour); } catch { structure = undefined; }
    const model: ModelField = { bytes: value.bytes, refused: value.refused, ...value.bytes ? {} : { data: value.data }, ...structure !== undefined ? { structure } : {} };
    fields[`model:${sub.id}`] = { value: model, content: structure !== undefined ? `structure:${sub.id}:${structure}` : `version:${head.version}` };
    const live = liveValues(picture);
    const held = state.get(`live:${sub.id}`) as { live: LiveMap | null; content: string } | undefined;
    const kept = held && sameLive(held.live, live) ? held : { live, content: content() };
    state.set(`live:${sub.id}`, kept);
    fields[`live:${sub.id}`] = { value: live, content: kept.content };
  });
  return fields;
}

export function fieldsView(structureOf: (product: string | undefined) => Structure | undefined,
  next?: (product: string | undefined, panel: FollowedPanel, hour: number) => number | undefined): LaneView {
  return {
    next: next && ((scene, hour) => {
      let soonest = Infinity;
      scene.subs.forEach((sub, at) => {
        const value = scene.pictures[at]!.parts[0]!.value;
        if (value.kind !== 'panel' || value.refused !== null) return;
        const change = next(sub.product, { data: value.data, refused: null }, hour);
        if (change !== undefined && change < soonest) soonest = change;
      });
      return soonest;
    }),
    paint: (input, state, content) => ({ surfaces: [], fields: fieldsOf(input, input.hour, structureOf, state, content), motion: false }),
  };
}
