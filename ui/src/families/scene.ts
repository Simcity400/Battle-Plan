import type { LiveValues } from '../charts/live';
import { repliedCells } from '../dashboard/follow/liveCells.ts';
import type { Fact, Picture } from '../dashboard/follow/facts.ts';
import type { FollowedAnswer } from '../dashboard/follow/message';
import type { Scene, Value as LaneValue } from '../draw/lane.ts';
import { sectionLiveValues } from '../draw/views/fields.ts';
import { artUrlForHandle, laneArtDemand, needsArtResolution, type ArtResolutions } from './art/provider.ts';
import { artHandleKey, flagArtKey } from './art/types.ts';
import { createBoundedReadView } from './boundedView.ts';
import type { ArtHandle, ArtProvider, ArtResult, BoundedReadView, BoundedSeries, DisplayedHour, EntityKind, FieldStatus, IdentityProvider, IdentityResult } from './contract.ts';
import type { IdentityIndex } from './identity/index.ts';
import { buildRowSourceFromHead, frozenAnswer, type DatedLogRow, type PagedRows, type RowWindow, type TableRowSource, type TailValidator } from './table/rowSource.ts';

export interface SceneSources {
  readonly sources: Readonly<Record<string, number>>;
  readonly staticPanels?: Readonly<Record<string, unknown>>;
  readonly staticShifts?: Readonly<Record<string, number>>;
  readonly staticAnswers?: Readonly<Record<string, FollowedAnswer>>;
  readonly series?: ReadonlyMap<string, ReadonlyMap<string, BoundedSeries>>;
  readonly hovers?: Readonly<Record<string, { readonly hour: number; readonly data: unknown }>>;
  readonly kind?: string;
  readonly pagedRows?: PagedRows;
  readonly window?: RowWindow;
  readonly validateTail?: TailValidator;
}

export interface SceneReaders {
  readonly identity?: IdentityProvider;
  readonly art?: ArtProvider;
  readonly index?: IdentityIndex | null;
  readonly artCacheRevision?: number;
  readonly artResolutions?: ArtResolutions;
  readonly artRevision?: number;
}

export interface CapturedSceneReads {
  readonly whoReads: readonly [EntityKind, string, DisplayedHour, IdentityResult][];
  readonly artReads: readonly [ArtHandle, ArtResult, string | null][];
  readonly identityDemands: readonly [string, SceneIdentityDemand][];
  readonly urls: readonly string[];
  readonly asks: readonly [string, ArtHandle][];
}

export interface SceneReadRecorder {
  readonly readers: SceneReaders;
  capture<T>(fn: () => T): { readonly value: T; readonly reads: CapturedSceneReads };
  replay(captured: CapturedSceneReads): void;
}

const sceneRecorders = new WeakMap<BoundedReadView, SceneReadRecorder>();

export function sceneReadRecorder(view: BoundedReadView): SceneReadRecorder | undefined {
  return sceneRecorders.get(view);
}

export type SceneIdentityDemand = readonly [kind: EntityKind, key: string, hour: number];

export interface SceneProjection<TModel> {
  readonly model: TModel;
  readonly view: BoundedReadView;
  readonly identityDemands: readonly SceneIdentityDemand[];
  readonly artUrls: readonly string[];
  readonly artAsks: readonly ArtHandle[];
  readonly boundary: number;
  readonly reads: SceneReads;
}

export interface SceneReads {
  readonly identity: number;
  readonly art: number;
  readonly window: boolean;
  unchanged(readers: SceneReaders): boolean;
}

type Live = LiveValues<number | bigint | string>;

interface DemandSink {
  readonly identity: Map<string, SceneIdentityDemand>;
  readonly urls: Set<string>;
  readonly asks: Map<string, ArtHandle>;
}

function readScene(
  scene: Scene,
  sources: SceneSources,
  hour: number,
  readers: SceneReaders | undefined,
  demands: DemandSink | null,
  noteWindow?: () => void,
  captureSinks?: readonly DemandSink[],
): BoundedReadView {
  let byName: Map<string, Picture<LaneValue>> | null = null;
  const pictureOf = (name: string): Picture<LaneValue> | undefined => {
    if (!byName) {
      byName = new Map();
      for (const [source, subId] of Object.entries(sources.sources)) {
        const at = scene.subs.findIndex(sub => sub.id === subId);
        const picture = at < 0 ? undefined : scene.pictures[at];
        if (picture) byName.set(source, picture);
      }
    }
    return byName.get(name);
  };
  const revisions: Record<string, number> = {};
  for (const name of Object.keys(sources.sources)) {
    const rev = pictureOf(name)?.parts[0]?.version;
    if (rev !== undefined) revisions[name] = rev;
  }
  const liveCache = new Map<string, Live | null>();
  const rowsCache = new Map<string, TableRowSource<DatedLogRow> | undefined>();
  return createBoundedReadView({
    hour,
    revisions,
    ...(sources.series ? { series: sources.series } : {}),
    ...(sources.hovers ? { hovers: new Map(Object.entries(sources.hovers)) } : {}),
    ...(sources.window ? { rowWindow: sources.window } : {}),
    ...(noteWindow ? { onRowWindow: noteWindow } : {}),
    ...(readers?.identity ? { identity: readers.identity } : {}),
    ...(readers?.art ? { art: readers.art } : {}),
    ...(demands
      ? {
          onIdentityDemand: (kind: EntityKind, key: string, atHour: number) => {
            const demandKey = `${kind}\u0000${key}\u0000${atHour}`;
            const entry: SceneIdentityDemand = [kind, key, atHour];
            if (!demands.identity.has(demandKey)) demands.identity.set(demandKey, entry);
            if (captureSinks) {
              for (let i = 0; i < captureSinks.length; i++) {
                const sink = captureSinks[i]!;
                if (!sink.identity.has(demandKey)) sink.identity.set(demandKey, entry);
              }
            }
          },
          onArtDemand: (handle: ArtHandle, result?: ArtResult) => {
            if (result?.status === 'ready' && result.url) {
              demands.urls.add(result.url);
              if (captureSinks) for (let i = 0; i < captureSinks.length; i++) captureSinks[i]!.urls.add(result.url);
              return;
            }
            laneArtDemand(handle, readers?.index, readers?.artResolutions, demands.urls, demands.asks);
            if (captureSinks) {
              for (let i = 0; i < captureSinks.length; i++) {
                const sink = captureSinks[i]!;
                laneArtDemand(handle, readers?.index, readers?.artResolutions, sink.urls, sink.asks);
              }
            }
          },
        }
      : {}),
    ...(sources.staticShifts ? { panelShifts: sources.staticShifts } : {}),
    ensureNonLive: () => {
      const answers = new Map<string, FollowedAnswer | undefined>();
      const panels = new Map<string, unknown>();
      const refused = new Map<string, string>();
      if (sources.staticAnswers) for (const [name, answer] of Object.entries(sources.staticAnswers)) answers.set(name, answer);
      if (sources.staticPanels) for (const [name, data] of Object.entries(sources.staticPanels)) panels.set(name, data);
      for (const name of Object.keys(sources.sources)) {
        const head = pictureOf(name)?.parts[0]?.value;
        if (!head) continue;
        if (head.kind === 'answer') answers.set(name, head.answer);
        else if (head.kind === 'panel') {
          if (head.refused === null) panels.set(name, head.data);
          else if (!frozenAnswer(sources.pagedRows?.[name], hour)) refused.set(name, head.refused);
        }
      }
      return { panels, answers, refused };
    },
    resolveLive: name => {
      if (liveCache.has(name)) return liveCache.get(name);
      if (sources.staticPanels && Object.hasOwn(sources.staticPanels, name)) {
        const live = repliedCells(sources.staticPanels[name]).read();
        liveCache.set(name, live);
        return live;
      }
      if (!Object.hasOwn(sources.sources, name)) return undefined;
      const picture = pictureOf(name);
      const head = picture?.parts[0]?.value;
      if (!picture || !head || head.kind !== 'panel' || head.refused !== null) return undefined;
      const live = sectionLiveValues(picture);
      liveCache.set(name, live);
      return live;
    },
    resolveRows: name => {
      if (rowsCache.has(name)) return rowsCache.get(name);
      const snapshot = sources.pagedRows?.[name];
      const identity = `${sources.kind ?? ''}:${name}`;
      let rows: TableRowSource<DatedLogRow> | undefined;
      if (sources.staticPanels && Object.hasOwn(sources.staticPanels, name)) {
        rows = buildRowSourceFromHead(identity, sources.staticPanels[name], undefined, hour, snapshot, sources.validateTail) ?? undefined;
      } else if (Object.hasOwn(sources.sources, name)) {
        const head = pictureOf(name)?.parts[0]?.value;
        if (head?.kind === 'panel') rows = buildRowSourceFromHead(identity, head.refused === null ? head.data : undefined, head.refused ?? undefined, hour, snapshot, sources.validateTail) ?? undefined;
      }
      rowsCache.set(name, rows);
      return rows;
    },
  });
}

export function sceneReadView(scene: Scene, sources: SceneSources, hour: number, readers?: SceneReaders): BoundedReadView {
  return readScene(scene, sources, hour, readers, null);
}

export function projectScene<TModel>(
  scene: Scene,
  sources: SceneSources,
  hour: number,
  readers: SceneReaders,
  project: (view: BoundedReadView) => TModel,
): SceneProjection<TModel> {
  const demands = { identity: new Map<string, SceneIdentityDemand>(), urls: new Set<string>(), asks: new Map<string, ArtHandle>() };
  const whoReads: [EntityKind, string, DisplayedHour, IdentityResult][] = [];
  const readKinds = new Set<EntityKind>();
  const artReads: [ArtHandle, ArtResult, string | null][] = [];
  const flagArtReads = new Map<string, [ArtHandle, ArtResult, string | null]>();
  const otherArtReads: [ArtHandle, ArtResult, string | null][] = [];
  let recording = true;
  const inner = readers.identity;
  const innerArt = readers.art;
  let prevIndex = readers.index ?? null;
  let prevFlagVersion = prevIndex?.flagRevision() ?? null;
  let prevArtCacheRevision = readers.artCacheRevision;
  let prevArtResolutions = readers.artResolutions;
  const recorded: SceneReaders = {
    ...readers,
    ...(inner
      ? {
          identity: {
            ...(inner.demandOwner ? { demandOwner: inner.demandOwner } : {}),
            resolve: (kind, key, atHour) => {
              const result = inner.resolve(kind, key, atHour);
              if (recording) {
                readKinds.add(kind);
                whoReads.push([kind, key, atHour, result]);
              }
              return result;
            },
          } satisfies IdentityProvider,
        }
      : {}),
    ...(innerArt
      ? {
          art: {
            ...(innerArt.demandOwner ? { demandOwner: innerArt.demandOwner } : {}),
            resolve: handle => {
              const result = innerArt.resolve(handle);
              if (recording) {
                const url = result.status === 'ready' ? result.url : artUrlForHandle(handle, readers.index, readers.artResolutions);
                const entry: [ArtHandle, ArtResult, string | null] = [handle, result, url];
                artReads.push(entry);
                if (handle.kind === 'flag') flagArtReads.set(flagArtKey(handle.size, handle.choice), entry);
                else otherArtReads.push(entry);
              }
              return result;
            },
          } satisfies ArtProvider,
        }
      : {}),
  };
  let windowRead = false;
  const captureStack: DemandSink[] = [];
  const view = readScene(scene, sources, hour, recorded, demands, () => { windowRead = true; }, captureStack);
  sceneRecorders.set(view, {
    readers,
    capture<T>(fn: () => T) {
      const whoStart = whoReads.length;
      const artStart = artReads.length;
      const sink: DemandSink = { identity: new Map(), urls: new Set(), asks: new Map() };
      captureStack.push(sink);
      try {
        const value = fn();
        return {
          value,
          reads: {
            whoReads: whoReads.slice(whoStart),
            artReads: artReads.slice(artStart),
            identityDemands: [...sink.identity.entries()],
            urls: [...sink.urls],
            asks: [...sink.asks.entries()],
          },
        };
      } finally {
        captureStack.pop();
      }
    },
    replay(captured: CapturedSceneReads) {
      if (!recording) return;
      for (let i = 0; i < captured.whoReads.length; i++) whoReads.push(captured.whoReads[i]!);
      for (let i = 0; i < captured.artReads.length; i++) artReads.push(captured.artReads[i]!);
      for (let i = 0; i < captured.identityDemands.length; i++) {
        const [idKey, entry] = captured.identityDemands[i]!;
        if (!demands.identity.has(idKey)) demands.identity.set(idKey, entry);
        for (let s = 0; s < captureStack.length; s++) {
          const sink = captureStack[s]!;
          if (!sink.identity.has(idKey)) sink.identity.set(idKey, entry);
        }
      }
      for (let i = 0; i < captured.urls.length; i++) {
        const url = captured.urls[i]!;
        demands.urls.add(url);
        for (let s = 0; s < captureStack.length; s++) captureStack[s]!.urls.add(url);
      }
      for (let i = 0; i < captured.asks.length; i++) {
        const [askKey, handle] = captured.asks[i]!;
        demands.asks.set(askKey, handle);
        for (let s = 0; s < captureStack.length; s++) captureStack[s]!.asks.set(askKey, handle);
      }
    },
  });
  const model = project(view);
  recording = false;
  for (const [handle] of artReads) if (needsArtResolution(handle)) demands.asks.set(artHandleKey(handle), handle);
  let boundary = Infinity;
  for (const read of whoReads) if (read[2] === view.hour && read[3].nextChangeAt < boundary) boundary = read[3].nextChangeAt;
  const checkArtEntry = (next: SceneReaders, [handle, was, url]: [ArtHandle, ArtResult, string | null]): boolean => {
    const resolved = next.art!.resolve(handle);
    const nextUrl = resolved.status === 'ready' ? resolved.url : artUrlForHandle(handle, next.index, next.artResolutions);
    return sameArt(was, resolved) && url === nextUrl;
  };
  return {
    model,
    view,
    identityDemands: [...demands.identity.values()],
    artUrls: [...demands.urls],
    artAsks: [...demands.asks.values()],
    boundary,
    reads: {
      identity: whoReads.length,
      art: artReads.length,
      window: windowRead,
      unchanged: next => {
        if (!next.identity !== !inner || !next.art !== !innerArt) return false;
        const baseIndex = prevIndex;
        const sameScope = baseIndex !== null && next.index != null && baseIndex.sameScope(next.index);
        if (whoReads.length > 0) {
          let allUnchanged = sameScope;
          if (sameScope) {
            for (const kind of readKinds) {
              const before = baseIndex.kindRevision(kind);
              const after = next.index!.kindRevision(kind);
              if (before === null || after === null || before !== after) {
                allUnchanged = false;
                break;
              }
            }
          }
          if (!allUnchanged) {
            for (const [kind, key, atHour, was] of whoReads) {
              if (sameScope) {
                const before = baseIndex.kindRevision(kind);
                const after = next.index!.kindRevision(kind);
                if (before !== null && after !== null && before === after) continue;
              }
              if (!sameIdentity(was, next.identity!.resolve(kind, key, atHour))) return false;
            }
          }
        }
        const sameArtCache = prevArtCacheRevision !== undefined && prevArtCacheRevision === next.artCacheRevision;
        if (flagArtReads.size > 0) {
          const delta = sameScope && sameArtCache && prevFlagVersion !== null ? next.index!.flagsChangedSince(prevFlagVersion) : null;
          if (delta !== null) {
            for (let i = 0; i < delta.length; i++) {
              const entry = flagArtReads.get(delta[i]!);
              if (entry && !checkArtEntry(next, entry)) return false;
            }
          } else {
            for (const entry of flagArtReads.values()) {
              if (!checkArtEntry(next, entry)) return false;
            }
          }
        }
        if (otherArtReads.length > 0 && !(sameArtCache && prevArtResolutions === next.artResolutions)) {
          for (const entry of otherArtReads) {
            if (!checkArtEntry(next, entry)) return false;
          }
        }
        prevIndex = next.index ?? null;
        prevFlagVersion = prevIndex?.flagRevision() ?? null;
        prevArtCacheRevision = next.artCacheRevision;
        prevArtResolutions = next.artResolutions;
        return true;
      },
    },
  };
}

function sameField<T>(a: FieldStatus<T> | undefined, b: FieldStatus<T> | undefined): boolean {
  if (a === b) return true;
  if (!a || !b || a.status !== b.status) return false;
  if (a.status === 'ready') return a.value === (b as typeof a).value;
  if (a.status === 'absent') return a.reason === (b as typeof a).reason;
  if (a.status === 'failed') return a.reason === (b as typeof a).reason && a.retryable === (b as typeof a).retryable;
  return true;
}

function sameIdentity(a: IdentityResult, b: IdentityResult): boolean {
  return a.nextChangeAt === b.nextChangeAt
    && a.validInterval[0] === b.validInterval[0]
    && a.validInterval[1] === b.validInterval[1]
    && sameField(a.holder, b.holder)
    && sameField(a.tag, b.tag)
    && sameField(a.name, b.name)
    && sameField(a.colour, b.colour)
    && sameField(a.flagChoice, b.flagChoice);
}

function sameArt(a: ArtResult, b: ArtResult): boolean {
  if (a === b) return true;
  if (a.status !== b.status) return false;
  switch (a.status) {
    case 'noArtExpected':
    case 'pending':
      return true;
    case 'unavailable':
      return a.reason === (b as typeof a).reason;
    case 'confirmedMissing': {
      const right = b as typeof a;
      return a.reason === right.reason
        && a.file === right.file
        && sameShallow(a.origin, right.origin);
    }
    case 'failed': {
      const right = b as typeof a;
      return a.reason === right.reason && a.retryable === right.retryable;
    }
    case 'refused': {
      const right = b as typeof a;
      return a.reason === right.reason
        && a.construct === right.construct
        && a.file === right.file
        && sameShallow(a.origin, right.origin);
    }
    case 'ready': {
      const right = b as typeof a;
      return a.url === right.url
        && a.assetId === right.assetId
        && a.file === right.file
        && a.bitmap === right.bitmap
        && sameShallow(a.origin, right.origin)
        && sameShallow(a.frame, right.frame)
        && sameShallow(a.provenance, right.provenance);
    }
  }
}

function sameShallow(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (!a || !b || typeof a !== 'object' || typeof b !== 'object') return false;
  const left = a as Readonly<Record<string, unknown>>;
  const right = b as Readonly<Record<string, unknown>>;
  let leftCount = 0;
  for (const key in left) {
    if (!Object.hasOwn(left, key)) continue;
    leftCount++;
    if (!Object.hasOwn(right, key) || left[key] !== right[key]) return false;
  }
  let rightCount = 0;
  for (const key in right) {
    if (Object.hasOwn(right, key)) rightCount++;
  }
  return leftCount === rightCount;
}

function sameBytes(a: Uint8Array, b: Uint8Array): boolean {
  if (a === b) return true;
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

function sameValue(a: LaneValue, b: LaneValue): boolean {
  if (a === b) return true;
  if (a.kind === 'panel' && b.kind === 'panel') return a.refused === b.refused && (a.bytes && b.bytes ? sameBytes(a.bytes, b.bytes) : a.data === b.data);
  if (a.kind === 'seed' && b.kind === 'seed') return a.values.length === b.values.length && a.values.every((value, at) => value === b.values[at]);
  return a.kind === 'answer' && b.kind === 'answer' && a.answer === b.answer;
}

function sameFact(a: Fact<LaneValue>, b: Fact<LaneValue>): boolean {
  return a === b || (a.refused === b.refused && a.cells.length === b.cells.length && a.cells.every((cell, at) => cell === b.cells[at]) && sameValue(a.value, b.value));
}

export function samePictures(prev: readonly (readonly Fact<LaneValue>[])[], pictures: readonly Picture<LaneValue>[], identical = false): boolean {
  if (prev.length !== pictures.length) return false;
  for (let i = 0; i < pictures.length; i++) {
    const a = prev[i]!;
    const b = pictures[i]!.parts;
    if (a.length !== b.length) return false;
    for (let j = 0; j < b.length; j++) if (identical ? a[j] !== b[j] : !sameFact(a[j]!, b[j]!)) return false;
  }
  return true;
}

export function snapshotPictures(pictures: readonly Picture<LaneValue>[]): readonly (readonly Fact<LaneValue>[])[] {
  return pictures.map(picture => picture.parts.slice());
}
