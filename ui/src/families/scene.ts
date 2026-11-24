import type { LiveValues } from '../charts/live';
import { repliedCells } from '../dashboard/follow/liveCells.ts';
import type { Picture } from '../dashboard/follow/facts.ts';
import type { FollowedAnswer } from '../dashboard/follow/message';
import type { Scene, Value as LaneValue } from '../draw/lane.ts';
import { sectionLiveValues } from '../draw/views/fields.ts';
import { laneArtDemand, type ArtResolutions } from './art/provider.ts';
import { createBoundedReadView } from './boundedView.ts';
import type { ArtHandle, ArtProvider, BoundedReadView, BoundedSeries, EntityKind, IdentityProvider } from './contract.ts';
import type { IdentityIndex } from './identity/index.ts';

export interface SceneSources {
  readonly sources: Readonly<Record<string, number>>;
  readonly staticPanels?: Readonly<Record<string, unknown>>;
  readonly staticAnswers?: Readonly<Record<string, FollowedAnswer>>;
  readonly series?: ReadonlyMap<string, ReadonlyMap<string, BoundedSeries>>;
  readonly hovers?: ReadonlyMap<string, { readonly hour: number; readonly data: unknown }>;
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
}

type Live = LiveValues<number | bigint | string>;

function readScene(
  scene: Scene,
  sources: SceneSources,
  hour: number,
  readers: SceneReaders | undefined,
  demands: { readonly identity: Map<string, SceneIdentityDemand>; readonly urls: Set<string>; readonly asks: Map<string, ArtHandle> } | null,
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
  return createBoundedReadView({
    hour,
    revisions,
    ...(sources.series ? { series: sources.series } : {}),
    ...(sources.hovers ? { hovers: sources.hovers } : {}),
    ...(readers?.identity ? { identity: readers.identity } : {}),
    ...(readers?.art ? { art: readers.art } : {}),
    ...(demands
      ? {
          onIdentityDemand: (kind: EntityKind, key: string, atHour: number) => {
            demands.identity.set(`${kind}\u0000${key}\u0000${atHour}`, [kind, key, atHour]);
          },
          onArtDemand: (handle: ArtHandle) => laneArtDemand(handle, readers?.index, readers?.artResolutions, demands.urls, demands.asks),
        }
      : {}),
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
          else refused.set(name, head.refused);
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
  const view = readScene(scene, sources, hour, readers, demands);
  const model = project(view);
  return {
    model,
    view,
    identityDemands: [...demands.identity.values()],
    artUrls: [...demands.urls],
    artAsks: [...demands.asks.values()],
  };
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

export function stringifyTooltipPayload(payload: unknown): string {
  return JSON.stringify(payload, (_k, v) => (typeof v === 'bigint' ? `${v}n` : v));
}
