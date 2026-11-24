import type { FamilyArtLoader } from '../artLoader.ts';
import { lookAt, nextLookChange, type CountryLooksMap } from '../looks.ts';
import type {
  ArtHandle,
  ArtProvider,
  ArtResult,
  BoundedReadView,
  DisplayedHour,
  EntityKind,
  IdentityProvider,
  IdentityResult,
} from '../plan.ts';
import type { BarsRowMedia } from './model.ts';

export function createLooksIdentityProvider(
  looks: CountryLooksMap | undefined,
  colours?: Readonly<Record<string, string>>,
): IdentityProvider {
  return {
    resolve(kind: EntityKind, key: string, atHour: DisplayedHour): IdentityResult {
      if (kind !== 'country' || !looks) {
        return {
          kind,
          key,
          atHour,
          holder: { status: 'pending' },
          name: { status: 'pending' },
          validInterval: [atHour, atHour],
          nextChangeAt: Infinity,
        };
      }
      const runs = looks[key];
      const look = lookAt(runs, atHour);
      const nextAt = nextLookChange({ [key]: runs ?? [] }, atHour);
      const validHi = Number.isFinite(nextAt) ? nextAt - 1 : atHour;
      const color = colours?.[key];
      if (!look) {
        return {
          kind,
          key,
          atHour,
          holder: { status: 'ready', value: key },
          tag: { status: 'ready', value: key },
          name: { status: 'absent', reason: 'Country name not recorded at this hour' },
          ...(color ? { colour: { status: 'ready', value: color } } : { colour: { status: 'absent', reason: 'Country colour not recorded' } }),
          flagChoice: { status: 'absent', reason: 'Country flag not recorded' },
          validInterval: [atHour, validHi],
          nextChangeAt: nextAt,
        };
      }
      const cleanName = look.name?.trim();
      return {
        kind,
        key,
        atHour,
        holder: { status: 'ready', value: key },
        tag: { status: 'ready', value: key },
        name: cleanName
          ? { status: 'ready', value: cleanName }
          : { status: 'absent', reason: look.status ?? 'Country name not recorded' },
        ...(color ? { colour: { status: 'ready', value: color } } : { colour: { status: 'absent', reason: 'Country colour not recorded' } }),
        flagChoice: look.flag
          ? { status: 'ready', value: look.flag }
          : { status: 'absent', reason: look.status ?? 'Flag artwork not recorded' },
        validInterval: [atHour, validHi],
        nextChangeAt: nextAt,
      };
    },
  };
}

export function createLoaderArtProvider(loader: FamilyArtLoader): ArtProvider {
  return {
    resolve(handle: ArtHandle): ArtResult {
      const url = handle.path ?? handle.choice;
      if (!url) return { status: 'none' };
      const existing = loader.get(url);
      if (existing === undefined) {
        loader.requestVisible([url]);
        return { status: 'pending' };
      }
      if (existing === null) {
        return { status: 'pending' };
      }
      return { status: 'ready', assetId: url, origin: 'current-installation', bitmap: existing };
    },
  };
}

export interface ResolvedCountryPresentation {
  readonly label: string;
  readonly labelMissing: boolean;
  readonly color?: string;
  readonly media: BarsRowMedia;
}

export function resolveCountryPresentation(
  readView: BoundedReadView,
  tag: string,
  fallbackColor?: string,
  dimmed?: boolean,
): ResolvedCountryPresentation {
  const identity = readView.who('country', tag);
  const labelMissing = identity.name.status === 'absent';
  const label = identity.name.status === 'ready' ? identity.name.value : tag;
  const color = identity.colour?.status === 'ready' ? identity.colour.value : fallbackColor;
  let media: BarsRowMedia;
  if (identity.flagChoice?.status === 'ready') {
    const choice = identity.flagChoice.value;
    const art = readView.art({ scope: 'flag', choice, path: choice });
    if (art.status === 'missing') {
      media = { kind: 'flag', state: 'missing', tag, reason: art.reason, dimmed };
    } else if (art.status === 'refused') {
      media = { kind: 'flag', state: 'refused', tag, reason: art.reason, dimmed };
    } else if (art.status === 'failed') {
      media = { kind: 'flag', state: 'failed', tag, reason: art.reason, dimmed };
    } else if (art.status === 'none') {
      media = { kind: 'flag', state: 'none', tag, dimmed };
    } else {
      media = { kind: 'flag', state: 'ready', iconUrl: choice, tag, dimmed };
    }
  } else if (identity.flagChoice?.status === 'absent') {
    media = { kind: 'flag', state: 'missing', tag, reason: identity.flagChoice.reason, dimmed };
  } else {
    media = { kind: 'flag', state: 'pending', tag, dimmed };
  }
  return { label, labelMissing, color, media };
}
