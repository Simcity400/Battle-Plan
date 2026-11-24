import { liveKey } from '../../charts/live.ts';
import { flagArtHandle, flagChoiceArt } from '../../families/art/provider.ts';
import { unavailableArt } from '../../families/art/types.ts';
import type { ArtHandle, ArtResult } from '../../families/contract.ts';
import type { HourRanges } from '../../families/holders.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import type { BoundedReadView, DeclaredCountryFilter, FamilyPlanContext, SourcePlan } from '../../families/plan.ts';
import { formatDateHover, formatDateOnly } from '../../families/valueFormat.ts';
import type { CipherCounts, CipherPair, CipherStatus, Country, DecryptionReply, Observation } from '../../generated/dashboard';
import {
  activeBonusDays,
  amountText,
  cipherStatus,
  pairKey,
  weekChangeText,
  validateDecryption,
  type CipherNumbers,
  type CountrySide,
} from './decryptionData.ts';

export interface DecryptionFilterOptions {
  readonly filter: DeclaredCountryFilter;
  readonly filterAll: boolean;
  readonly side: CountrySide;
}

export interface ResolvedCountry {
  readonly key: string;
  readonly tag: string;
  readonly name: string;
  readonly colour?: string;
  readonly flagHandle: ArtHandle | null;
  readonly flag: ArtResult;
}

export type DecryptionFrame =
  | { readonly kind: 'refused'; readonly reason: string }
  | { readonly kind: 'pending' }
  | {
      readonly kind: 'empty';
      readonly empty: string;
      readonly emptyDetail?: string;
      readonly emptyKind: 'empty' | 'unavailable';
      readonly summaryText?: string;
      readonly warnings: readonly string[];
    }
  | {
      readonly kind: 'pairs';
      readonly reply: DecryptionReply;
      readonly live: ReturnType<BoundedReadView['live']>;
      readonly pairs: readonly CipherPair[];
      readonly byStatus: Readonly<Record<CipherStatus, readonly CipherPair[]>>;
      readonly summaryText: string;
      readonly notes: readonly string[];
      readonly country: (country: Country, hour?: number) => ResolvedCountry;
    };

const within = (ranges: HourRanges | undefined, hour: number) =>
  !!ranges?.some(([first, last]) => first <= hour && hour <= last);

export function decryptionNextChange(filter: DeclaredCountryFilter, hour: number): number {
  let low = 0, high = filter.bounds.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (filter.bounds[mid]! <= hour) low = mid + 1;
    else high = mid;
  }
  return filter.bounds[low] ?? Infinity;
}

function resolveCountry(readView: BoundedReadView, country: Country, hour: number): ResolvedCountry {
  const tag = country.tag;
  if (!tag) {
    return {
      key: String(country.id),
      tag: '?',
      name: `Country ${country.id}`,
      flagHandle: null,
      flag: unavailableArt('Country tag not recorded'),
    };
  }
  const who = readView.who('country', tag, hour);
  const name = who.name.status === 'ready' ? who.name.value : tag;
  const colour = who.colour?.status === 'ready' ? who.colour.value : undefined;
  let flagHandle: ArtHandle | null = null;
  if (who.flagChoice?.status === 'ready') flagHandle = flagArtHandle(who.flagChoice.value, 'panel');
  const flag: ArtResult = flagChoiceArt(readView, who.flagChoice);
  return { key: String(country.id), tag, name, ...(colour ? { colour } : {}), flagHandle, flag };
}

export function readCipherNumbers(live: ReturnType<BoundedReadView['live']>, pair: CipherPair): CipherNumbers {
  const key = pairKey(pair);
  const rawAmt = live?.get(liveKey(key, 'amount'));
  const rawBonus = live?.get(liveKey(key, 'bonusDays'));
  const amount = typeof rawAmt === 'bigint' ? rawAmt : typeof rawAmt === 'number' && Number.isSafeInteger(rawAmt) ? BigInt(rawAmt) : undefined;
  const bonusDays = typeof rawBonus === 'number' ? rawBonus : typeof rawBonus === 'bigint' ? Number(rawBonus) : undefined;
  return {
    amount,
    bonusDays,
    missing: amount === undefined ? (live?.missing(liveKey(key, 'amount')) || 'Not recorded') : '',
  };
}

function observationHover(observation: Observation | null, unknown: string): string {
  if (!observation) return unknown;
  return `${observation.upperBound ? 'By ' : ''}${formatDateHover(observation.hour)}`;
}

export function pairTooltipItems(
  pair: CipherPair,
  reply: DecryptionReply,
  numbers: CipherNumbers,
  owner: ResolvedCountry,
  target: ResolvedCountry,
  percent: number | null,
): TooltipListItem[] {
  const attemptUnknown = reply.attemptsUnknown === null ? 'Not recorded' : 'Not known';
  const bonus = activeBonusDays(pair, numbers);
  return [
    { label: 'Codebreaker', value: owner.name, swatch: owner.colour },
    { label: 'Target', value: target.name, swatch: target.colour },
    {
      label: 'Progress',
      ...(percent === null
        ? { missingReason: pair.thresholdUnknown ?? (numbers.missing || 'Progress not recorded') }
        : { value: `${percent}%` }),
    },
    ...(percent === null && numbers.amount !== undefined ? [{ label: 'Recorded amount', value: amountText(numbers) }] : []),
    ...(pair.broken === true ? [] : [{
      label: 'Last 7 days',
      ...(pair.weekChangeTenths === null
        ? { missingReason: pair.weekChangeUnknown ?? 'Not recorded' }
        : { value: weekChangeText(pair.weekChangeTenths) }),
    }]),
    { label: 'Started', value: observationHover(pair.started, attemptUnknown) },
    ...(pair.broken === true ? [{ label: 'Broken', value: observationHover(pair.brokenAt, attemptUnknown) }] : []),
    ...(bonus !== null ? [{ label: 'Active bonus', value: `${bonus} bonus ${bonus === 1 ? 'day' : 'days'}` }] : []),
    { label: 'Status', value: cipherStatus(pair, reply), divider: true },
  ];
}

export function brokenText(observation: Observation | null): string | null {
  if (!observation) return null;
  return `${observation.upperBound ? 'By ' : ''}${formatDateOnly(observation.hour)}`;
}

export function countsText(counts: CipherCounts, partial = false): string {
  return [
    `${counts.broken} broken`,
    `${counts.decrypting} decrypting`,
    ...(counts.paused ? [`${counts.paused} paused`] : []),
    ...(counts.breakUnknown ? [`${counts.breakUnknown} not known`] : []),
    ...(partial ? ['Partial'] : []),
  ].join(' \u00b7 ');
}

export function readDecryption(readView: BoundedReadView, options: DecryptionFilterOptions): DecryptionFrame {
  const refused = readView.refused('main');
  if (refused) return { kind: 'refused', reason: refused };
  const raw = readView.panel<unknown>('main');
  if (!raw) return { kind: 'pending' };
  const reply = validateDecryption(raw);
  if (reply.missing !== null) {
    return {
      kind: 'empty',
      empty: 'Decryption data is unavailable at this time.',
      emptyDetail: reply.missing,
      emptyKind: 'unavailable',
      warnings: reply.warnings,
    };
  }
  const unrecordedFilter = options.filter.unrecorded.find(
    ([first, last]) => first <= readView.hour && readView.hour <= last,
  )?.[2] ?? null;
  if (unrecordedFilter) {
    return { kind: 'empty', empty: unrecordedFilter, emptyKind: 'unavailable', warnings: reply.warnings };
  }
  const passing = options.filter.passing;
  const pairs = passing
    ? reply.pairs.filter(pair => {
        const tag = (options.side === 'decryptor' ? pair.owner : pair.target).tag;
        return tag !== null && within(passing[tag], readView.hour);
      })
    : reply.pairs;
  if (!pairs.length) {
    return {
      kind: 'empty',
      empty: options.filterAll ? 'No ciphers being decrypted yet.' : 'No matching decryption pairs at this time.',
      emptyKind: 'empty',
      summaryText: countsText({ broken: 0, decrypting: 0, paused: 0, breakUnknown: 0 }, reply.attemptsUnknown !== null),
      warnings: reply.warnings,
    };
  }
  const cache = new Map<string, ResolvedCountry>();
  const country = (c: Country, hour: number = readView.hour): ResolvedCountry => {
    const at = Math.min(hour, readView.hour);
    const key = `${c.id}@${at}`;
    let held = cache.get(key);
    if (!held) {
      held = resolveCountry(readView, c, at);
      cache.set(key, held);
    }
    return held;
  };
  const byStatus: Record<CipherStatus, CipherPair[]> = { broken: [], decrypting: [], paused: [], breakUnknown: [] };
  for (const pair of pairs) byStatus[pair.status].push(pair);
  const counts: CipherCounts = options.filterAll
    ? reply.counts
    : { broken: byStatus.broken.length, decrypting: byStatus.decrypting.length, paused: byStatus.paused.length, breakUnknown: byStatus.breakUnknown.length };
  return {
    kind: 'pairs',
    reply,
    live: readView.live('main'),
    pairs,
    byStatus,
    summaryText: countsText(counts, reply.attemptsUnknown !== null),
    notes: reply.attemptsUnknown ? [reply.attemptsUnknown] : [],
    country,
  };
}

export function decryptionSources(ctx: FamilyPlanContext): SourcePlan {
  return {
    owner: `decryption:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage: { mode: 'population', form: 'all' },
        product: 'decryption',
        followQuery: { panel: 'decryption', branchId: ctx.branch.id },
        params: { branchId: ctx.branch.id },
        validate: validateDecryption,
      },
    }],
  };
}

export function decryptionFilterOptions(ctx: FamilyPlanContext): DecryptionFilterOptions {
  return {
    filter: ctx.countries.filter,
    filterAll: ctx.countries.all,
    side: ctx.options.countrySide === 'target' ? 'target' : 'decryptor',
  };
}
