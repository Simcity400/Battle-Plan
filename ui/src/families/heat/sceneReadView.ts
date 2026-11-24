import type { LiveValues } from '../../charts/live';
import { NOT_LIVE_CELL, NOT_RECORDED, repliedCells } from '../../dashboard/follow/liveCells.ts';
import type { Picture } from '../../dashboard/follow/facts.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { Scene, Value as LaneValue } from '../../draw/lane.ts';
import { liveValues, type LiveMap } from '../../draw/views/fields.ts';
import { createBoundedReadView } from '../boundedView.ts';
import type { ArtDemand, ArtHandle, ArtProvider, BoundedReadView, EntityKind, IdentityDemand, IdentityProvider } from '../contract.ts';

export interface HeatSceneSources {
  readonly sources: Readonly<Record<string, number>>;
  readonly staticPanels?: Readonly<Record<string, unknown>>;
  readonly staticAnswers?: Readonly<Record<string, FollowedAnswer>>;
}

export interface HeatReadViewProviders {
  readonly identity?: IdentityProvider;
  readonly onIdentityDemand?: IdentityDemand;
  readonly art?: ArtProvider;
  readonly onArtDemand?: ArtDemand;
}

export function liveAdapter(map: LiveMap | null): LiveValues<number | bigint | string> | null {
  if (!map) return null;
  return {
    get: (key: string) => {
      const value = map.get(key);
      return typeof value === 'number' || typeof value === 'bigint' ? value : undefined;
    },
    raw: (key: string) => {
      const value = map.get(key);
      return typeof value === 'bigint'
        ? value
        : typeof value === 'number' && Number.isSafeInteger(value)
          ? BigInt(value)
          : undefined;
    },
    missing: (key: string) => {
      if (!map.has(key)) return NOT_LIVE_CELL;
      const value = map.get(key);
      return typeof value === 'string' && value.length > 0 ? value : NOT_RECORDED;
    },
  };
}

function answerOf(value: LaneValue): FollowedAnswer | undefined {
  return value.kind === 'answer' ? value.answer : undefined;
}

export function samePictures(
  prev: readonly (readonly unknown[])[],
  pictures: readonly Picture<LaneValue>[],
): boolean {
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

export function readViewFromHeatScene(
  scene: Scene,
  binding: HeatSceneSources,
  hour: number,
  providers?: HeatReadViewProviders,
): BoundedReadView {
  const liveCache = new Map<string, LiveValues<number | bigint | string> | null>();
  const resolveLive = (name: string): LiveValues<number | bigint | string> | null | undefined => {
    if (liveCache.has(name)) return liveCache.get(name);
    if (binding.staticPanels && Object.hasOwn(binding.staticPanels, name)) {
      const val = repliedCells(binding.staticPanels[name]).read();
      liveCache.set(name, val);
      return val;
    }
    if (Object.hasOwn(binding.sources, name)) {
      const subId = binding.sources[name]!;
      const at = scene.subs.findIndex(sub => sub.id === subId);
      const picture = at < 0 ? undefined : scene.pictures[at];
      const head = picture?.parts[0]?.value;
      if (head && head.kind === 'panel' && head.refused === null) {
        const val = liveAdapter(liveValues(picture!));
        liveCache.set(name, val);
        return val;
      }
    }
    return undefined;
  };
  const revisions: Record<string, number> = {};
  for (const [name, subId] of Object.entries(binding.sources)) {
    const at = scene.subs.findIndex(sub => sub.id === subId);
    const rev = at < 0 ? undefined : scene.pictures[at]?.parts[0]?.version;
    if (rev !== undefined) revisions[name] = rev;
  }
  return createBoundedReadView({
    hour,
    revisions,
    identity: providers?.identity,
    onIdentityDemand: providers?.onIdentityDemand,
    art: providers?.art,
    onArtDemand: providers?.onArtDemand,
    ensureNonLive: () => {
      const answers = new Map<string, FollowedAnswer | undefined>();
      const panels = new Map<string, unknown>();
      const refused = new Map<string, string>();
      if (binding.staticAnswers) {
        for (const [name, answer] of Object.entries(binding.staticAnswers)) answers.set(name, answer);
      }
      if (binding.staticPanels) {
        for (const [name, data] of Object.entries(binding.staticPanels)) panels.set(name, data);
      }
      for (const [name, subId] of Object.entries(binding.sources)) {
        const at = scene.subs.findIndex(sub => sub.id === subId);
        const picture = at < 0 ? undefined : scene.pictures[at];
        const head = picture?.parts[0]?.value;
        if (!head) continue;
        if (head.kind === 'answer') answers.set(name, answerOf(head));
        else if (head.kind === 'panel') {
          if (head.refused === null) panels.set(name, head.data);
          else refused.set(name, head.refused);
        }
      }
      return { panels, answers, refused };
    },
    resolveLive,
  });
}

export type SceneIdentityDemand = readonly [kind: EntityKind, key: string, hour: number];

export interface SceneProjection<TModel> {
  readonly model: TModel;
  readonly view: BoundedReadView;
  readonly identityDemands: readonly SceneIdentityDemand[];
  readonly artUrls: readonly string[];
}

export interface SceneProjectionProviders {
  readonly identity?: IdentityProvider;
  readonly art?: ArtProvider;
  readonly artUrl: (handle: ArtHandle) => string | null;
}

export function projectFromScene<TModel>(
  scene: Scene,
  binding: HeatSceneSources,
  hour: number,
  providers: SceneProjectionProviders,
  project: (view: BoundedReadView) => TModel,
): SceneProjection<TModel> {
  const identity = new Map<string, SceneIdentityDemand>();
  const art = new Set<string>();
  const view = readViewFromHeatScene(scene, binding, hour, {
    identity: providers.identity,
    onIdentityDemand: (kind, key, atHour) => { identity.set(`${kind}\u0000${key}\u0000${atHour}`, [kind, key, atHour]); },
    art: providers.art,
    onArtDemand: handle => {
      const url = providers.artUrl(handle);
      if (url) art.add(url);
    },
  });
  const model = project(view);
  return { model, view, identityDemands: [...identity.values()], artUrls: [...art] };
}

export function stringifyTooltipPayload(payload: unknown): string {
  return JSON.stringify(payload, (_k, v) => (typeof v === 'bigint' ? `${v}n` : v));
}
