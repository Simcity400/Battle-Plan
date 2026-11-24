import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import { answerGroup, CellGroup, sectionGroups, sectionOf, sectionValues, valuedGroup, type GroupValues, type LiveSection } from '../../dashboard/follow/liveCells.ts';
import { changedAnswerRows, serializeAnswer } from '../../dashboard/follow/tables.ts';
import { parseAnswer, type FollowedPanel } from '../../dashboard/follow/message.ts';
import type { Picture } from '../../dashboard/follow/facts';
import type { LaneView, Painted, Scene, Value } from '../lane.ts';

export type Structure = (panel: FollowedPanel, hour: number) => string | undefined;
export type LiveMap = Map<string, number | bigint | string>;

export interface ModelField { bytes: Uint8Array | null; data?: unknown; refused: string | null; structure?: string }

const groups = new WeakMap<object, GroupValues>();
const pictureLiveCache = new WeakMap<Picture<Value>, LiveValues<number> | null>();
const liveMeta = new WeakMap<object, { readonly picture: Picture<Value>; readonly section: LiveSection }>();
const sectionRevIndex = new WeakMap<LiveSection, readonly (readonly (readonly string[] | undefined)[])[]>();
const EMPTY_STRING_SET: ReadonlySet<string> = new Set();

function reverseRowIndex(section: LiveSection): readonly (readonly (readonly string[] | undefined)[])[] {
  let held = sectionRevIndex.get(section);
  if (held) return held;
  const byGroup: (string[] | undefined)[][] = section.groups.map(() => []);
  const { cells, rows } = section;
  for (let at = 0; at < cells.length;) {
    const rowId = rows[cells[at]!]!;
    const count = cells[at + 2]!;
    for (let s = 0; s < count; s++) {
      const base = at + 3 + s * 3;
      const g = cells[base]!;
      const k = cells[base + 1]!;
      let bucket = byGroup[g]![k];
      if (!bucket) byGroup[g]![k] = bucket = [rowId];
      else if (!bucket.includes(rowId)) bucket.push(rowId);
    }
    at += 3 + count * 3;
  }
  sectionRevIndex.set(section, held = byGroup);
  return held;
}

export function changedLiveRows(
  prev: LiveValues<number | bigint | string> | null | undefined,
  next: LiveValues<number | bigint | string> | null | undefined,
): ReadonlySet<string> | null {
  if (prev === next) return EMPTY_STRING_SET;
  if (!prev || !next) return null;
  const metaA = liveMeta.get(prev);
  const metaB = liveMeta.get(next);
  if (!metaA || !metaB || metaA.section !== metaB.section) return null;
  const picA = metaA.picture;
  const picB = metaB.picture;
  if (picA.parts[0] !== picB.parts[0] || picA.parts.length !== picB.parts.length) return null;
  const cells = picB.parts[0]!.cells;
  const rev = reverseRowIndex(metaB.section);
  const out = new Set<string>();
  for (let g = 0; g < metaB.section.groups.length; g++) {
    const followId = metaB.section.groups[g]!.follow;
    if (followId === undefined) continue;
    const at = cells.indexOf(followId);
    const pA = at < 0 ? undefined : picA.parts[at + 1]?.value;
    const pB = at < 0 ? undefined : picB.parts[at + 1]?.value;
    if (pA === pB) continue;
    if (!pA || !pB || pA.kind !== 'answer' || pB.kind !== 'answer' || pA.answer.kind !== 'rows' || pB.answer.kind !== 'rows') {
      return null;
    }
    const diff = changedAnswerRows(pA.answer, pB.answer);
    if (diff === null) return null;
    const groupRev = rev[g]!;
    for (let i = 0; i < diff.length; i++) {
      const ids = groupRev[diff[i]!];
      if (!ids) continue;
      for (let j = 0; j < ids.length; j++) out.add(ids[j]!);
    }
  }
  return out;
}

function groupOf(value: Value, spec: Parameters<typeof valuedGroup>[0]): GroupValues {
  if (value.kind === 'seed') return valuedGroup(spec, value.values);
  let held = groups.get(value);
  if (!held) {
    held = value.kind === 'answer' ? answerGroup(value.answer) : new CellGroup();
    groups.set(value, held);
  }
  return held;
}

export function sectionLiveValues(picture: Picture<Value>): LiveValues<number> | null {
  if (pictureLiveCache.has(picture)) return pictureLiveCache.get(picture)!;
  const head = picture.parts[0]!.value;
  if (head.kind !== 'panel' || head.refused !== null) {
    pictureLiveCache.set(picture, null);
    return null;
  }
  const section = sectionOf(head.data);
  if (!section) {
    pictureLiveCache.set(picture, null);
    return null;
  }
  const cells = picture.parts[0]!.cells;
  const live = sectionValues(section, sectionGroups(section, id => {
    const at = cells.indexOf(id), part = at < 0 ? undefined : picture.parts[at + 1];
    const spec = section.groups.find(group => group.follow === id);
    return part && spec ? groupOf(part.value, spec) : undefined;
  }));
  liveMeta.set(live, { picture, section });
  pictureLiveCache.set(picture, live);
  return live;
}

export function liveValues(picture: Picture<Value>): LiveMap | null {
  const head = picture.parts[0]!.value;
  if (head.kind !== 'panel' || head.refused !== null) return null;
  const section = sectionOf(head.data);
  if (!section) return null;
  const live = sectionLiveValues(picture);
  if (!live) return null;
  const out: LiveMap = new Map();
  for (let i = 0; i < section.cells.length; i += 3 + section.cells[i + 2]! * 3) {
    const key = liveKey(section.rows[section.cells[i]!]!, section.kinds[section.cells[i + 1]!]!.id);
    const num = live.get(key);
    if (num !== undefined) {
      if (Math.abs(num) >= 9_007_199_254_740_991) {
        const raw = live.raw?.(key);
        out.set(key, raw !== undefined && !Number.isSafeInteger(Number(raw)) ? raw : num);
      } else {
        out.set(key, num);
      }
    } else {
      const raw = live.raw?.(key);
      out.set(key, raw !== undefined ? raw : live.missing(key));
    }
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
      let body = head.value.body;
      if (!body) {
        if (head.value.answer.kind === 'notRecorded') body = new Uint8Array();
        else {
          body = serializeAnswer(head.value.answer);
          head.value.body = body;
          head.value.answer = parseAnswer(body);
        }
      }
      fields[`answer:${sub.id}`] = { value: body, content: `answer:${head.version}` };
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
