import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import { useContentRevision } from '../../dashboard/contentRevision.ts';
import type { PlaybackController } from '../../dashboard/playback';
import type { CountryFlagChoice, CountryFlagSize } from '../../generated/dashboard';
import { flagEntryToArt } from '../art/adapters.ts';
import type { ArtResolution } from '../art/types.ts';
import { PENDING_ART, missingArt } from '../art/types.ts';
import { identityBrokerFor } from './broker.ts';
import { IdentityIndex } from './index.ts';
import {
  identityKeyString,
  type CountryIdentityResult,
  type DrawnFlagEntry,
  type HourInterval,
  type IdentityAsk,
  type IdentityKey,
  type IdentityResult,
  type IdentityScope,
  type IdentitySnapshot,
} from './types.ts';

const NO_SUBSCRIBE = () => () => {};

export function useIdentityScope(
  client: DashboardClient,
  recording: Pick<DashboardSource, 'id' | 'revision'> | null | undefined,
  branchId: string | null | undefined,
  active = true,
  language?: string,
): IdentityScope | null {
  const content = useContentRevision(client, active && !!recording && !!branchId);
  return useMemo(
    () => (recording && branchId
      ? { sourceId: recording.id, revision: recording.revision, branchId, content, ...(language ? { language } : {}) }
      : null),
    [recording?.id, recording?.revision, branchId, content, language],
  );
}

export function useIdentity<K extends IdentityKey>(
  client: DashboardClient,
  recording: DashboardSource | null | undefined,
  scope: IdentityScope | null,
  key: K | null,
  hour: number | null,
  options: {
    readonly active?: boolean;
    readonly flagSize?: CountryFlagSize;
    readonly needFlag?: boolean;
    readonly branchBounds?: HourInterval;
  } = {},
): { readonly result: IdentityResult<K> | null; readonly flag: DrawnFlagEntry | undefined; readonly flagArt: ArtResolution } {
  const active = options.active ?? true;
  const flagSize: CountryFlagSize = options.flagSize ?? 'panel';
  const needFlag = options.needFlag ?? false;
  const broker = identityBrokerFor(client);
  const [owner] = useState(() => ({}));
  const boundsKey = options.branchBounds ? `${options.branchBounds[0]}:${options.branchBounds[1]}` : '';
  const bounds = useMemo(() => options.branchBounds, [boundsKey]);

  const subscribe = useCallback(
    (changed: () => void) => (active && scope ? broker.subscribe(scope, changed, recording ?? undefined, bounds) : NO_SUBSCRIBE()),
    [broker, active, scope, recording, bounds],
  );
  const version = useSyncExternalStore(
    subscribe,
    () => (scope ? broker.versionOf(scope) : 0),
    () => (scope ? broker.versionOf(scope) : 0),
  );

  const askKey = key && hour !== null && Number.isFinite(hour) ? `${identityKeyString(key)}@${Math.floor(hour)}` : '';
  useEffect(() => {
    if (!active || !scope || !key || hour === null || !Number.isFinite(hour)) {
      if (scope) broker.setDemand(owner, scope, null);
      return;
    }
    broker.setDemand(
      owner,
      scope,
      { asks: [{ key, hour: Math.floor(hour) }], needFlags: needFlag, flagSize },
      recording ?? undefined,
      bounds,
    );
    return () => { broker.setDemand(owner, scope, null); };
  }, [broker, owner, active, scope, askKey, needFlag, flagSize, recording, bounds]);

  return useMemo(() => {
    void version;
    if (!scope || !key || hour === null || !Number.isFinite(hour)) {
      return { result: null, flag: undefined, flagArt: PENDING_ART };
    }
    const result = broker.resolve(scope, key, Math.floor(hour), recording ?? undefined, bounds);
    let flag: DrawnFlagEntry | undefined;
    let flagArt: ArtResolution = PENDING_ART;
    if (key.kind === 'country') {
      const country = result as CountryIdentityResult;
      if (country.kind === 'known') {
        if (country.flagChoice.kind === 'known') {
          flag = broker.flagFor(scope, flagSize, country.flagChoice.value);
          flagArt = flagEntryToArt(flag);
        } else if (country.flagChoice.kind === 'missing') {
          flagArt = missingArt(country.flagChoice.reason);
        } else if (country.flagChoice.kind === 'failed') {
          flagArt = { status: 'failed', reason: country.flagChoice.failure.message, retryable: true };
        }
      } else if (country.kind === 'absent') {
        flagArt = missingArt(country.reason);
      } else if (country.kind === 'failed') {
        flagArt = { status: 'failed', reason: country.failure.message, retryable: true };
      }
    }
    return { result, flag, flagArt };
  }, [broker, scope, key, hour, version, flagSize, recording, bounds]);
}

export function useIdentities(
  client: DashboardClient,
  recording: DashboardSource | null | undefined,
  scope: IdentityScope | null,
  asks: readonly IdentityAsk[],
  options: {
    readonly active?: boolean;
    readonly flagSize?: CountryFlagSize;
    readonly needFlags?: boolean;
    readonly needIdentities?: boolean;
    readonly needGroups?: boolean;
    readonly branchBounds?: HourInterval;
  } = {},
): {
  readonly index: IdentityIndex | null;
  readonly version: number;
  readonly resolve: <K extends IdentityKey>(key: K, hour: number) => IdentityResult<K>;
  readonly flagFor: (choice: CountryFlagChoice) => DrawnFlagEntry | undefined;
  readonly flagArtFor: (choice: CountryFlagChoice) => ArtResolution;
  readonly retry: () => void;
} {
  const active = options.active ?? true;
  const flagSize: CountryFlagSize = options.flagSize ?? 'panel';
  const needFlags = options.needFlags ?? false;
  const needIdentities = options.needIdentities ?? false;
  const needGroups = options.needGroups ?? false;
  const broker = identityBrokerFor(client);
  const [owner] = useState(() => ({}));
  const boundsKey = options.branchBounds ? `${options.branchBounds[0]}:${options.branchBounds[1]}` : '';
  const bounds = useMemo(() => options.branchBounds, [boundsKey]);

  const subscribe = useCallback(
    (changed: () => void) => (active && scope ? broker.subscribe(scope, changed, recording ?? undefined, bounds) : NO_SUBSCRIBE()),
    [broker, active, scope, recording, bounds],
  );
  const version = useSyncExternalStore(
    subscribe,
    () => (scope ? broker.versionOf(scope) : 0),
    () => (scope ? broker.versionOf(scope) : 0),
  );

  const asksRef = useRef(asks);
  asksRef.current = asks;
  const asksSig = asks.map(ask => `${identityKeyString(ask.key)}@${Math.floor(ask.hour)}`).join('\u0001');

  useEffect(() => {
    if (!active || !scope) {
      if (scope) broker.setDemand(owner, scope, null);
      return;
    }
    broker.setDemand(
      owner,
      scope,
      { asks: asksRef.current, needFlags, flagSize, needIdentities, needGroups },
      recording ?? undefined,
      bounds,
    );
    return () => { broker.setDemand(owner, scope, null); };
  }, [broker, owner, active, scope, asksSig, needFlags, flagSize, needIdentities, needGroups, recording, bounds]);

  const index = useMemo(
    () => (scope ? broker.indexOf(scope, recording ?? undefined, bounds) : null),
    [broker, scope, recording, bounds, version],
  );

  const resolve = useCallback(
    <K extends IdentityKey>(key: K, hour: number): IdentityResult<K> => (
      index ? index.resolve(key, hour) : ({ kind: 'pending' } as IdentityResult<K>)
    ),
    [index, version],
  );

  const flagFor = useCallback(
    (choice: CountryFlagChoice): DrawnFlagEntry | undefined => (scope ? broker.flagFor(scope, flagSize, choice) : undefined),
    [broker, scope, flagSize, version],
  );

  const flagArtFor = useCallback(
    (choice: CountryFlagChoice): ArtResolution => flagEntryToArt(flagFor(choice)),
    [flagFor],
  );

  const retry = useCallback(() => {
    if (scope) broker.retry(scope);
  }, [broker, scope]);

  return { index, version, resolve, flagFor, flagArtFor, retry };
}

export function useIdentityAtHead(
  client: DashboardClient,
  recording: DashboardSource | null | undefined,
  scope: IdentityScope | null,
  clock: PlaybackController,
  keys: readonly IdentityKey[],
  trackHour: (clockHour: number) => number | null,
  options: {
    readonly active?: boolean;
    readonly follow?: boolean;
    readonly flagSize?: CountryFlagSize;
    readonly needFlags?: boolean;
    readonly branchBounds?: HourInterval;
  } = {},
): {
  readonly hour: number | null;
  readonly results: readonly IdentityResult[];
  readonly flagFor: (choice: CountryFlagChoice) => DrawnFlagEntry | undefined;
  readonly flagArtFor: (choice: CountryFlagChoice) => ArtResolution;
  readonly retry: () => void;
} {
  const active = options.active ?? true;
  const follow = options.follow ?? true;
  const broker = identityBrokerFor(client);
  const boundsKey = options.branchBounds ? `${options.branchBounds[0]}:${options.branchBounds[1]}` : '';
  const bounds = useMemo(() => options.branchBounds, [boundsKey]);
  const keysRef = useRef(keys);
  keysRef.current = keys;
  const trackHourRef = useRef(trackHour);
  trackHourRef.current = trackHour;

  const held = useRef<{ readonly key: string; readonly hour: number | null }>({ key: '', hour: null });

  const hasKeys = keys.length > 0;
  const subscribe = useCallback((changed: () => void) => {
    if (!active || !follow || !scope || !hasKeys) return NO_SUBSCRIBE();
    const stopClock = clock.subscribeFrame(changed);
    const stopAnchor = clock.subscribe(changed);
    const stopBroker = broker.subscribe(scope, changed, recording ?? undefined, bounds);
    return () => { stopClock(); stopAnchor(); stopBroker(); };
  }, [active, follow, scope, hasKeys, clock, broker, recording, bounds]);

  const readHour = (): number | null => {
    if (!active || !scope || !keysRef.current.length) {
      if (held.current.key === 'idle') return held.current.hour;
      held.current = { key: 'idle', hour: null };
      return null;
    }
    const raw = trackHourRef.current(clock.getHour());
    const at = raw !== null && Number.isFinite(raw) ? Math.floor(raw) : null;
    if (at === null) {
      if (held.current.key === 'null') return held.current.hour;
      held.current = { key: 'null', hour: null };
      return null;
    }
    const index = broker.indexOf(scope, recording ?? undefined, bounds);
    let intervalFirst = -Infinity;
    let intervalLast = Infinity;
    let allCovered = true;
    const parts: string[] = [];
    for (const key of keysRef.current) {
      const res = index.resolve(key, at);
      if (res.kind === 'known' || res.kind === 'absent') {
        intervalFirst = Math.max(intervalFirst, res.valid[0]);
        intervalLast = Math.min(intervalLast, res.valid[1]);
        parts.push(`${res.kind}:${res.valid[0]}:${res.valid[1]}`);
      } else {
        allCovered = false;
        parts.push(`${res.kind}:${at}`);
      }
    }
    const signature = allCovered && intervalFirst <= intervalLast
      ? `run:${broker.versionOf(scope)}:${intervalFirst}:${intervalLast}:${parts.join('|')}`
      : `point:${broker.versionOf(scope)}:${at}:${parts.join('|')}`;
    if (held.current.key === signature && held.current.hour !== null && held.current.hour >= intervalFirst && held.current.hour <= intervalLast) {
      return held.current.hour;
    }
    held.current = { key: signature, hour: at };
    return at;
  };

  const hour = useSyncExternalStore(subscribe, readHour, readHour);
  const asks = useMemo<readonly IdentityAsk[]>(
    () => (hour === null ? [] : keys.map(key => ({ key, hour }))),
    [hour, keys],
  );
  const state = useIdentities(client, recording, scope, asks, {
    active: active && hour !== null,
    flagSize: options.flagSize,
    needFlags: options.needFlags,
    branchBounds: bounds,
  });
  const results = useMemo(
    () => (hour === null ? [] : keys.map(key => state.resolve(key, hour))),
    [hour, keys, state.resolve],
  );
  return { hour, results, flagFor: state.flagFor, flagArtFor: state.flagArtFor, retry: state.retry };
}

export function useIdentitySnapshot(
  client: DashboardClient,
  recording: DashboardSource | null | undefined,
  scope: IdentityScope | null,
  asks: readonly IdentityAsk[],
  options: {
    readonly active?: boolean;
    readonly flagSize?: CountryFlagSize;
    readonly needFlags?: boolean;
    readonly needIdentities?: boolean;
    readonly needGroups?: boolean;
    readonly branchBounds?: HourInterval;
  } = {},
): IdentitySnapshot | null {
  const broker = identityBrokerFor(client);
  const { version } = useIdentities(client, recording, scope, asks, options);
  return useMemo(() => {
    void version;
    return scope ? broker.snapshotOf(scope, recording ?? undefined, options.branchBounds) : null;
  }, [broker, scope, recording, options.branchBounds, version]);
}

export {
  countryHolderKey,
  countryKeyFromString,
  countrySlotKey,
  countryTagKey,
  designKey,
  factionKey,
  flagChoiceKey,
  leaderKey,
  regionKey,
  stateKey,
  type CountryIdentityKey,
  type CountryIdentityResult,
  type DrawnFlagEntry,
  type EntityIdentityResult,
  type FactionIdentityKey,
  type FactionIdentityResult,
  type IdentityAsk,
  type IdentityField,
  type IdentityKey,
  type IdentityResult,
  type IdentityScope,
  type IdentitySnapshot,
} from './types.ts';
