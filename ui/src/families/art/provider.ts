import type { DashboardSource } from '../../dashboard/client';
import type { Artwork, CountryFlagChoice } from '../../generated/dashboard';
import type { IdentityBroker } from '../identity/broker.ts';
import type { IdentityIndex } from '../identity/index.ts';
import { parseFlagChoiceKey, type DrawnFlagEntry, type IdentityScope } from '../identity/types.ts';
import type { ArtFlagSize, ArtHandle, ArtProvider, ArtResult, FieldStatus } from '../plan.ts';
import { flagEntryToArt } from './adapters.ts';
import type { DomArtCache } from './domCache.ts';
import type { LaneArtCache } from './laneCache.ts';
import {
  CURRENT_INSTALL_PROVENANCE,
  PENDING_ART,
  artHandleKey,
  defaultArtFrame,
  failedArt,
  readyArt,
  unavailableArt,
  type ArtResolution,
} from './types.ts';

export function flagArtHandle(flagChoice: string, size: ArtFlagSize = 'panel'): ArtHandle {
  return { kind: 'flag', choice: flagChoice, size };
}

export function flagChoiceArt(
  readView: { art(handle: ArtHandle): ArtResult },
  choice: FieldStatus<string> | undefined,
  size: ArtFlagSize = 'panel',
): ArtResult {
  if (!choice) return PENDING_ART;
  switch (choice.status) {
    case 'ready':
      return readView.art(flagArtHandle(choice.value, size));
    case 'pending':
      return PENDING_ART;
    case 'absent':
      return unavailableArt(choice.reason);
    case 'failed':
      return failedArt(choice.reason, choice.retryable);
  }
}

export function productIconArt(
  readView: { art(handle: ArtHandle): ArtResult },
  url: string | null | undefined,
  missing: string | null | undefined,
): ArtResult {
  return url ? readView.art({ kind: 'url', url }) : unavailableArt(missing ?? 'The product drew no icon');
}

export function artworkArt(readView: { art(handle: ArtHandle): ArtResult }, artwork: Pick<Artwork, 'icon' | 'sprite'>): ArtResult {
  if (artwork.icon) return readView.art({ kind: 'url', url: artwork.icon });
  if (artwork.sprite) return readView.art({ kind: 'sprite', key: artwork.sprite, frame: 0 });
  return unavailableArt('The recording names no icon for this item');
}

export function urlArtHandle(url: string | null | undefined): ArtHandle | null {
  return url ? { kind: 'url', url } : null;
}

export type ArtUrlInput =
  | string
  | ArtHandle
  | ArtResult
  | { readonly iconUrl?: string | null; readonly art?: ArtResult }
  | null
  | undefined;

export function artUrlOf(
  input: ArtUrlInput,
  index?: IdentityIndex | null,
  resolutions?: ArtResolutions,
): string | null {
  if (!input) return null;
  if (typeof input === 'string') return input || null;
  if ('status' in input) return input.status === 'ready' && input.url ? input.url : null;
  if ('iconUrl' in input || 'art' in input) {
    if (input.iconUrl) return input.iconUrl;
    return input.art?.status === 'ready' && input.art.url ? input.art.url : null;
  }
  if ('kind' in input) return artUrlForHandle(input, index, resolutions);
  return null;
}

export function collectArtUrls(
  items: Iterable<ArtUrlInput>,
  index?: IdentityIndex | null,
  resolutions?: ArtResolutions,
): string[] {
  const urls = new Set<string>();
  for (const item of items) {
    const url = artUrlOf(item, index, resolutions);
    if (url) urls.add(url);
  }
  return [...urls];
}

export function mergeArtDemands(held: readonly ArtHandle[], asked: readonly ArtHandle[]): readonly ArtHandle[] {
  const seen = new Set(held.map(artHandleKey));
  let merged: ArtHandle[] | null = null;
  for (const handle of asked) {
    const key = artHandleKey(handle);
    if (seen.has(key)) continue;
    seen.add(key);
    (merged ??= [...held]).push(handle);
  }
  return merged ?? held;
}

function isDirectUrl(candidate: string): boolean {
  return candidate.startsWith('http://')
    || candidate.startsWith('https://')
    || candidate.startsWith('asset://')
    || candidate.startsWith('data:')
    || candidate.startsWith('blob:')
    || candidate.startsWith('/');
}

function directUrlOf(handle: ArtHandle): string | null {
  if (handle.kind === 'url') return handle.url || null;
  if (handle.kind === 'path' && isDirectUrl(handle.path)) return handle.path;
  return null;
}

export type ArtResolutions = Readonly<Record<string, ArtResolution>>;

export function needsArtResolution(handle: ArtHandle): boolean {
  return handle.kind === 'sprite' || handle.kind === 'portrait' || handle.kind === 'technology' || (handle.kind === 'path' && !isDirectUrl(handle.path));
}

export function artUrlForHandle(handle: ArtHandle, index?: IdentityIndex | null, resolutions?: ArtResolutions): string | null {
  const direct = directUrlOf(handle);
  if (direct) return direct;
  if (needsArtResolution(handle)) {
    const resolved = resolutions?.[artHandleKey(handle)];
    return resolved?.status === 'ready' ? resolved.url : null;
  }
  if (handle.kind === 'flag' && index) {
    const choice = parseFlagChoiceKey(handle.choice);
    if (choice) {
      const drawn = index.getDrawnFlag(handle.size, choice);
      const res = drawn?.image?.resolution;
      if (res) return res.status === 'ready' ? res.url : null;
      if (drawn?.url) return drawn.url;
    }
  }
  return null;
}

export function laneArtDemand(
  handle: ArtHandle,
  index: IdentityIndex | null | undefined,
  resolutions: ArtResolutions | undefined,
  urls: Set<string>,
  asks: Map<string, ArtHandle>,
): void {
  const url = artUrlForHandle(handle, index, resolutions);
  if (url) urls.add(url);
  else if (needsArtResolution(handle) ? !resolutions?.[artHandleKey(handle)] : undrawnFlag(handle, index)) {
    asks.set(artHandleKey(handle), handle);
  }
}

function undrawnFlag(handle: ArtHandle, index: IdentityIndex | null | undefined): boolean {
  if (handle.kind !== 'flag') return false;
  const choice = parseFlagChoiceKey(handle.choice);
  return choice !== null && index?.getDrawnFlag(handle.size, choice) === undefined;
}

export function requestDemandedFlags(
  broker: IdentityBroker,
  scope: IdentityScope,
  handles: readonly ArtHandle[],
  recording?: DashboardSource,
): void {
  const bySize = new Map<ArtFlagSize, CountryFlagChoice[]>();
  for (const handle of handles) {
    if (handle.kind !== 'flag') continue;
    const choice = parseFlagChoiceKey(handle.choice);
    if (!choice) continue;
    let list = bySize.get(handle.size);
    if (!list) bySize.set(handle.size, list = []);
    list.push(choice);
  }
  for (const [size, choices] of bySize) broker.requestFlagChoices(scope, size, choices, recording);
}

function resolutionFromHandle(
  handle: ArtHandle,
  lookupFlag: (choiceKey: string, size: ArtFlagSize) => DrawnFlagEntry | undefined,
  resolutions?: ArtResolutions,
): ArtResolution {
  if (needsArtResolution(handle)) return resolutions?.[artHandleKey(handle)] ?? PENDING_ART;
  const direct = directUrlOf(handle);
  if (direct) {
    const frameIndex = handle.kind === 'url' || handle.kind === 'path' ? handle.frame : undefined;
    const frameCount = handle.kind === 'path' && handle.frames !== undefined ? handle.frames : 1;
    return readyArt(direct, {
      frame: frameIndex !== undefined ? defaultArtFrame(0, 0, frameIndex, frameCount) : defaultArtFrame(),
      provenance: CURRENT_INSTALL_PROVENANCE,
    });
  }
  if (handle.kind === 'flag') {
    return flagEntryToArt(lookupFlag(handle.choice, handle.size));
  }
  return PENDING_ART;
}

export interface LaneArtProviderOptions {
  readonly index?: IdentityIndex | null;
  readonly resolutions?: ArtResolutions;
}

export function createLaneArtProvider(
  cache: LaneArtCache,
  options: LaneArtProviderOptions = {},
): ArtProvider {
  const lookupFlag = (choiceKey: string, size: ArtFlagSize): DrawnFlagEntry | undefined => {
    if (!options.index) return undefined;
    const choice = parseFlagChoiceKey(choiceKey);
    return choice ? options.index.getDrawnFlag(size, choice) : undefined;
  };
  return {
    demandOwner: 'view',
    resolve(handle: ArtHandle): ArtResult {
      return withoutBitmap(cache.resolve(resolutionFromHandle(handle, lookupFlag, options.resolutions)));
    },
  };
}

function withoutBitmap(result: ArtResult): ArtResult {
  if (result.status !== 'ready' || result.bitmap === undefined) return result;
  const { bitmap: _decoded, ...address } = result;
  return address;
}

export interface DomArtProviderOptions {
  readonly broker?: IdentityBroker;
  readonly scope?: IdentityScope | null;
  readonly recording?: DashboardSource;
  readonly resolutions?: ArtResolutions;
}

export function createDomArtProvider(
  cache: DomArtCache,
  options: DomArtProviderOptions = {},
): ArtProvider {
  const lookupFlag = (choiceKey: string, size: ArtFlagSize): DrawnFlagEntry | undefined => {
    if (!options.broker || !options.scope) return undefined;
    const choice = parseFlagChoiceKey(choiceKey);
    if (!choice) return undefined;
    const entry = options.broker.flagFor(options.scope, size, choice);
    if (!entry) {
      options.broker.requestFlagChoices(options.scope, size, [choice], options.recording);
    }
    return entry;
  };
  return {
    demandOwner: 'view',
    resolve(handle: ArtHandle): ArtResult {
      return withoutBitmap(cache.resolve(resolutionFromHandle(handle, lookupFlag, options.resolutions)));
    },
  };
}
