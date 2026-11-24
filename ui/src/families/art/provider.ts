import type { DashboardSource } from '../../dashboard/client';
import type { CountryFlagSize } from '../../generated/dashboard';
import type { IdentityBroker } from '../identity/broker.ts';
import type { IdentityIndex } from '../identity/index.ts';
import { parseFlagChoiceKey, type DrawnFlagEntry, type IdentityScope } from '../identity/types.ts';
import type { ArtHandle, ArtProvider, ArtResult } from '../plan.ts';
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

export function flagArtHandle(flagChoice: string, size: CountryFlagSize = 'panel'): ArtHandle {
  return { scope: size === 'shield' ? 'flag:shield' : 'flag:panel', choice: flagChoice };
}

function flagSizeFromScope(scope: string): CountryFlagSize | null {
  if (scope === 'flag' || scope === 'flag:panel') return 'panel';
  if (scope === 'flag:shield') return 'shield';
  return null;
}

function directUrlOf(handle: ArtHandle): string | null {
  const candidate = handle.scope === 'url' ? (handle.path ?? handle.choice) : handle.path;
  if (!candidate) return null;
  if (
    handle.scope === 'url'
    || candidate.startsWith('http://')
    || candidate.startsWith('https://')
    || candidate.startsWith('asset://')
    || candidate.startsWith('data:')
    || candidate.startsWith('blob:')
    || candidate.startsWith('/')
  ) {
    return candidate;
  }
  return null;
}

export function artUrlForHandle(handle: ArtHandle, index?: IdentityIndex | null): string | null {
  const direct = directUrlOf(handle);
  if (direct) return direct;
  const flagSize = flagSizeFromScope(handle.scope);
  if (flagSize && index) {
    const choice = parseFlagChoiceKey(handle.choice);
    if (choice) {
      const drawn = index.getDrawnFlag(choice);
      const res = drawn?.image?.resolution;
      if (res) return res.status === 'ready' ? res.url : null;
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
    if (custom.status !== 'pending' || (!flagSizeFromScope(handle.scope) && !directUrlOf(handle))) {
      return custom;
    }
  }
  const direct = directUrlOf(handle);
  if (direct) {
    return readyArt(direct, {
      frame: handle.frame !== undefined ? defaultArtFrame(0, 0, handle.frame, 1) : defaultArtFrame(),
      provenance: CURRENT_INSTALL_PROVENANCE,
    });
  }
  const flagSize = flagSizeFromScope(handle.scope);
  if (flagSize) {
    return flagEntryToArt(lookupFlag(handle.choice, flagSize));
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
