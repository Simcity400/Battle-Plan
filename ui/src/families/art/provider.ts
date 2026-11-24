import type { DashboardSource } from '../../dashboard/client';
import type { CountryFlagSize } from '../../generated/dashboard';
import type { IdentityBroker } from '../identity/broker.ts';
import type { IdentityIndex } from '../identity/index.ts';
import { parseFlagChoiceKey, type DrawnFlagEntry, type IdentityScope } from '../identity/types.ts';
import type { ArtFlagSize, ArtHandle, ArtProvider, ArtResult } from '../plan.ts';
import { flagEntryToArt } from './adapters.ts';
import type { DomArtCache } from './domCache.ts';
import type { LaneArtCache } from './laneCache.ts';
import {
  CURRENT_INSTALL_PROVENANCE,
  PENDING_ART,
  defaultArtFrame,
  readyArt,
  type ArtResolution,
} from './types.ts';

export function flagArtHandle(flagChoice: string, size: ArtFlagSize = 'panel', maxEdge?: number): ArtHandle {
  return { kind: 'flag', choice: flagChoice, size, ...(maxEdge !== undefined ? { maxEdge } : {}) };
}

function brokerFlagSize(size: ArtFlagSize): CountryFlagSize {
  return size === 'shield' ? 'shield' : 'panel';
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

export function artUrlForHandle(handle: ArtHandle, index?: IdentityIndex | null): string | null {
  const direct = directUrlOf(handle);
  if (direct) return direct;
  if (handle.kind === 'flag' && index) {
    const choice = parseFlagChoiceKey(handle.choice);
    if (choice) {
      const drawn = index.getDrawnFlag(choice);
      if (drawn?.url) return drawn.url;
    }
  }
  return null;
}

function resolutionFromHandle(
  handle: ArtHandle,
  lookupFlag: (choiceKey: string, size: CountryFlagSize) => DrawnFlagEntry | undefined,
  resolveHandle?: (handle: ArtHandle) => ArtResolution,
): ArtResolution {
  if (resolveHandle) {
    const custom = resolveHandle(handle);
    if (custom.status !== 'pending' || (handle.kind !== 'flag' && !directUrlOf(handle))) {
      return custom;
    }
  }
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
    return flagEntryToArt(lookupFlag(handle.choice, brokerFlagSize(handle.size)), CURRENT_INSTALL_PROVENANCE);
  }
  return PENDING_ART;
}

export interface LaneArtProviderOptions {
  readonly index?: IdentityIndex | null;
  readonly resolveHandle?: (handle: ArtHandle) => ArtResolution;
}

export function createLaneArtProvider(
  cache: LaneArtCache,
  options: LaneArtProviderOptions = {},
): ArtProvider {
  const lookupFlag = (choiceKey: string): DrawnFlagEntry | undefined => {
    if (!options.index) return undefined;
    const choice = parseFlagChoiceKey(choiceKey);
    return choice ? options.index.getDrawnFlag(choice) : undefined;
  };
  return {
    demandOwner: 'view',
    resolve(handle: ArtHandle): ArtResult {
      return cache.resolve(resolutionFromHandle(handle, lookupFlag, options.resolveHandle));
    },
  };
}

export interface DomArtProviderOptions {
  readonly broker?: IdentityBroker;
  readonly scope?: IdentityScope | null;
  readonly recording?: DashboardSource;
  readonly resolveHandle?: (handle: ArtHandle) => ArtResolution;
}

export function createDomArtProvider(
  cache: DomArtCache,
  options: DomArtProviderOptions = {},
): ArtProvider {
  const lookupFlag = (choiceKey: string, size: CountryFlagSize): DrawnFlagEntry | undefined => {
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
      const result = cache.resolve(resolutionFromHandle(handle, lookupFlag, options.resolveHandle));
      if (result.status === 'ready') {
        const { bitmap: _domImage, ...withoutBitmap } = result;
        return withoutBitmap;
      }
      return result;
    },
  };
}
