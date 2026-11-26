import type { LiveValues } from '../charts/live';
import { repliedCells } from '../dashboard/follow/liveCells.ts';
import type { Picture } from '../dashboard/follow/facts.ts';
import type { FollowedAnswer } from '../dashboard/follow/message';
import type { Scene, Value as LaneValue } from '../draw/lane.ts';
import { sectionLiveValues } from '../draw/views/fields.ts';
import { artUrlForHandle, laneArtDemand, needsArtResolution, type ArtResolutions } from './art/provider.ts';
import { artHandleKey } from './art/types.ts';
import { createBoundedReadView } from './boundedView.ts';
import type { ArtHandle, ArtProvider, ArtResult, BoundedReadView, BoundedSeries, DisplayedHour, EntityKind, FieldStatus, IdentityProvider, IdentityResult } from './contract.ts';
import type { IdentityIndex } from './identity/index.ts';
import { buildRowSourceFromHead, frozenAnswer, frozenHour, type DatedLogRow, type PagedRows, type RowWindow, type TableRowSource } from './table/rowSource.ts';

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
}

export interface SceneReaders {
  readonly identity?: IdentityProvider;
  readonly art?: ArtProvider;
  readonly index?: IdentityIndex | null;
  readonly artResolutions?: ArtResolutions;
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

function readScene(
  scene: Scene,
  sources: SceneSources,
  hour: number,
  readers: SceneReaders | undefined,
  demands: { readonly identity: Map<string, SceneIdentityDemand>; readonly urls: Set<string>; readonly asks: Map<string, ArtHandle> } | null,
  noteWindow?: () => void,
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
  const headHolds = (name: string): boolean => {
    const snapshot = sources.pagedRows?.[name];
    const frozen = frozenHour(snapshot, hour);
    if (frozen === null) return true;
    if (frozenAnswer(snapshot, hour)) return false;
    const head = pictureOf(name)?.parts[0];
    return head !== undefined && head.h <= frozen && frozen <= head.c;
  };
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
            demands.identity.set(`${kind}\u0000${key}\u0000${atHour}`, [kind, key, atHour]);
          },
          onArtDemand: (handle: ArtHandle, result?: ArtResult) => {
            if (result?.status === 'ready' && result.url) {
              demands.urls.add(result.url);
              return;
            }
            laneArtDemand(handle, readers?.index, readers?.artResolutions, demands.urls, demands.asks);
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
          else if (headHolds(name)) refused.set(name, head.refused);
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
        rows = buildRowSourceFromHead(identity, sources.staticPanels[name], undefined, hour, snapshot) ?? undefined;
      } else if (Object.hasOwn(sources.sources, name)) {
        const head = pictureOf(name)?.parts[0]?.value;
        const holds = headHolds(name);
        if (head?.kind === 'panel') rows = buildRowSourceFromHead(identity, holds && head.refused === null ? head.data : undefined, holds ? head.refused ?? undefined : undefined, hour, snapshot) ?? undefined;
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
  const artReads: [ArtHandle, ArtResult, string | null][] = [];
  let recording = true;
  const inner = readers.identity;
  const innerArt = readers.art;
  const recorded: SceneReaders = {
    ...readers,
    ...(inner
      ? {
          identity: {
            ...(inner.demandOwner ? { demandOwner: inner.demandOwner } : {}),
            resolve: (kind, key, atHour) => {
              const result = inner.resolve(kind, key, atHour);
              if (recording) whoReads.push([kind, key, atHour, result]);
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
                artReads.push([handle, result, url]);
              }
              return result;
            },
          } satisfies ArtProvider,
        }
      : {}),
  };
  let windowRead = false;
  const view = readScene(scene, sources, hour, recorded, demands, () => { windowRead = true; });
  const model = project(view);
  recording = false;
  for (const [handle] of artReads) if (needsArtResolution(handle)) demands.asks.set(artHandleKey(handle), handle);
  let boundary = Infinity;
  for (const read of whoReads) if (read[2] === view.hour && read[3].nextChangeAt < boundary) boundary = read[3].nextChangeAt;
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
        for (const [kind, key, atHour, was] of whoReads) {
          if (!sameIdentity(was, next.identity!.resolve(kind, key, atHour))) return false;
        }
        for (const [handle, was, url] of artReads) {
          const resolved = next.art!.resolve(handle);
          const nextUrl = resolved.status === 'ready' ? resolved.url : artUrlForHandle(handle, next.index, next.artResolutions);
          if (!sameArt(was, resolved) || url !== nextUrl) return false;
        }
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
  const left = a as unknown as Readonly<Record<string, unknown>>;
  const right = b as unknown as Readonly<Record<string, unknown>>;
  const keys = Object.keys(left).filter(key => key !== 'retry');
  if (keys.length !== Object.keys(right).filter(key => key !== 'retry').length) return false;
  for (const key of keys) if (!sameShallow(left[key], right[key])) return false;
  return true;
}

function sameShallow(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (!a || !b || typeof a !== 'object' || typeof b !== 'object') return false;
  const left = a as Readonly<Record<string, unknown>>;
  const right = b as Readonly<Record<string, unknown>>;
  const keys = Object.keys(left);
  return keys.length === Object.keys(right).length && keys.every(key => left[key] === right[key]);
}

export function samePictures(prev: readonly (readonly unknown[])[], pictures: readonly Picture<LaneValue>[]): boolean {
  if (prev.length !== pictures.length) return false;
  for (let i = 0; i < pictures.length; i++) {
    const a = prev[i]!;
    const b = pictures[i]!.parts;
    if (a.length !== b.length) return false;
    for (let j = 0; j < b.length; j++) if (a[j] !== b[j]) return false;
  }
  return true;
}

export function snapshotPictures(pictures: readonly Picture<LaneValue>[]): readonly (readonly unknown[])[] {
  return pictures.map(picture => picture.parts.slice());
}
