import { liveKey } from '../../charts/live.ts';
import { flagArtHandle } from '../../families/art/provider.ts';
import { missingArt, PENDING_ART, type ArtResult } from '../../families/art/types.ts';
import type { HourRanges } from '../../families/holders.ts';
import type { BoundedReadView, DeclaredCountryFilter } from '../../families/plan.ts';
import type { TooltipListItem } from '../../families/host/index.ts';
import type { MeterWidgetDeclaration } from '../../families/meter/declaration.ts';
import type {
  MeterChip,
  MeterChipLine,
  MeterFlag,
  MeterMatrixCell,
  MeterMatrixColumn,
  MeterMatrixRow,
  MeterModel,
  MeterProjection,
  MeterRow,
  MeterSection,
} from '../../families/meter/model.ts';
import { registerMeterProjection } from '../../families/meter/registry.ts';
import { formatDateHover, formatDateOnly } from '../../families/valueFormat.ts';
import type { CipherPair, Country, DecryptionReply, Observation } from '../../generated/dashboard';
import { OPTIONS } from './definition.ts';
import {
  activeBonusDays,
  amountText,
  cipherStatus,
  pairKey,
  percentOf,
  progressLabel,
  validateDecryption,
  workingStatus,
  type CipherNumbers,
  type CountrySide,
} from './decryptionData.ts';

export type DecryptionView = 'ciphers' | 'matrix';

export interface DecryptionProjectionOptions {
  readonly view: DecryptionView;
  readonly filter: DeclaredCountryFilter;
  readonly filterAll: boolean;
  readonly side: CountrySide;
}

const within = (ranges: HourRanges | undefined, hour: number) =>
  !!ranges?.some(([first, last]) => first <= hour && hour <= last);

function filterNext(filter: DeclaredCountryFilter, hour: number): number {
  let low = 0, high = filter.bounds.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (filter.bounds[mid]! <= hour) low = mid + 1;
    else high = mid;
  }
  return filter.bounds[low] ?? Infinity;
}

interface ResolvedCountry {
  readonly key: string;
  readonly tag: string;
  readonly name: string;
  readonly colour?: string;
  readonly flag: MeterFlag;
}

function resolveCountry(readView: BoundedReadView, country: Country): ResolvedCountry {
  const tag = country.tag;
  if (!tag) {
    return {
      key: String(country.id),
      tag: '?',
      name: `Country ${country.id}`,
      flag: { tag: '?', result: missingArt('Country tag not recorded') },
    };
  }
  const who = readView.who('country', tag, readView.hour);
  const name = who.name.status === 'ready' ? who.name.value : tag;
  const colour = who.colour?.status === 'ready' ? who.colour.value : undefined;
  let flagResult: ArtResult;
  if (who.flagChoice?.status === 'ready') {
    flagResult = readView.art(flagArtHandle(who.flagChoice.value, 'panel'));
  } else if (who.flagChoice?.status === 'absent') {
    flagResult = missingArt(who.flagChoice.reason);
  } else {
    flagResult = PENDING_ART;
  }
  return {
    key: String(country.id),
    tag,
    name,
    colour,
    flag: { tag, result: flagResult },
  };
}

function readCipherNumbers(
  live: ReturnType<BoundedReadView['live']>,
  pair: CipherPair,
): CipherNumbers {
  const key = pairKey(pair);
  const rawAmt = live?.get(liveKey(key, 'amount'));
  const rawBonus = live?.get(liveKey(key, 'bonusDays'));
  const amount = typeof rawAmt === 'number' ? rawAmt : typeof rawAmt === 'bigint' ? Number(rawAmt) : undefined;
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

function pairTooltipItems(
  pair: CipherPair,
  reply: DecryptionReply,
  numbers: CipherNumbers,
  owner: ResolvedCountry,
  target: ResolvedCountry,
  percent: number | null,
): TooltipListItem[] {
  const attemptUnknown = reply.attemptsUnknown === null ? 'Not recorded' : 'Not known';
  const bonus = activeBonusDays(pair, numbers);
  const items: TooltipListItem[] = [
    { label: 'Codebreaker', value: owner.name, swatch: owner.colour },
    { label: 'Target', value: target.name, swatch: target.colour },
    {
      label: 'Progress',
      ...(percent === null
        ? { missingReason: pair.thresholdUnknown ?? (numbers.missing || 'Progress not recorded') }
        : { value: `${percent}%` }),
    },
    ...(percent === null && numbers.amount !== undefined ? [{ label: 'Recorded amount', value: amountText(numbers) }] : []),
    { label: 'Started', value: observationHover(pair.started, attemptUnknown) },
    ...(pair.broken === true
      ? [{ label: 'Broken', value: observationHover(pair.brokenAt, attemptUnknown) }]
      : []),
    ...(bonus !== null ? [{ label: 'Active bonus', value: `${bonus} bonus ${bonus === 1 ? 'day' : 'days'}` }] : []),
    { label: 'Status', value: cipherStatus(pair, reply), divider: true },
  ];
  return items;
}

export const decryptionProjection: MeterProjection<DecryptionProjectionOptions> = registerMeterProjection({
  kind: 'decryption',
  nextChange(readView, options): number {
    return filterNext(options.filter, readView.hour);
  },
  project(readView, options): MeterModel {
    const refused = readView.refused('main');
    const layout = options.view === 'matrix' ? 'matrix' : 'ciphers';
    if (refused) return { hour: readView.hour, layout, refused };
    const raw = readView.panel<unknown>('main');
    if (!raw) return { hour: readView.hour, layout, empty: null };
    const reply = validateDecryption(raw);
    const live = readView.live('main');

    if (reply.missing !== null) {
      return {
        hour: readView.hour,
        layout,
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
      return {
        hour: readView.hour,
        layout,
        empty: unrecordedFilter,
        emptyKind: 'unavailable',
        warnings: reply.warnings,
      };
    }

    const passing = options.filter.passing;
    const filteredPairs = passing
      ? reply.pairs.filter(pair => {
          const tag = (options.side === 'decryptor' ? pair.owner : pair.target).tag;
          return tag !== null && within(passing[tag], readView.hour);
        })
      : reply.pairs;

    if (!filteredPairs.length) {
      return {
        hour: readView.hour,
        layout,
        summary: '0 broken \u00b7 0 decrypting',
        empty: options.filterAll
          ? 'No ciphers being decrypted yet.'
          : 'No matching decryption pairs at this time.',
        emptyKind: 'empty',
        warnings: reply.warnings,
      };
    }

    const countryCache = new Map<number, ResolvedCountry>();
    const countryOf = (c: Country): ResolvedCountry => {
      let held = countryCache.get(c.id);
      if (!held) {
        held = resolveCountry(readView, c);
        countryCache.set(c.id, held);
      }
      return held;
    };

    const brokenPairs: CipherPair[] = [];
    const activePairs: CipherPair[] = [];
    for (const pair of filteredPairs) {
      if (pair.broken === true) brokenPairs.push(pair);
      else activePairs.push(pair);
    }

    const summary = `${brokenPairs.length} broken \u00b7 ${activePairs.length} decrypting`;
    const notes = reply.attemptsUnknown ? [reply.attemptsUnknown] : [];

    if (layout === 'matrix') {
      const ownerStats = new Map<number, { country: ResolvedCountry; broken: number; decrypting: number }>();
      const targetStats = new Map<number, { country: ResolvedCountry; broken: number; decrypting: number }>();
      const pairMap = new Map<string, CipherPair>();

      for (const pair of filteredPairs) {
        pairMap.set(pairKey(pair), pair);
        const o = ownerStats.get(pair.owner.id) ?? { country: countryOf(pair.owner), broken: 0, decrypting: 0 };
        if (pair.broken === true) o.broken++;
        else o.decrypting++;
        ownerStats.set(pair.owner.id, o);

        const t = targetStats.get(pair.target.id) ?? { country: countryOf(pair.target), broken: 0, decrypting: 0 };
        if (pair.broken === true) t.broken++;
        else t.decrypting++;
        targetStats.set(pair.target.id, t);
      }

      const sortedOwners = [...ownerStats.entries()].sort((a, b) =>
        b[1].broken - a[1].broken
        || b[1].decrypting - a[1].decrypting
        || a[1].country.name.localeCompare(b[1].country.name),
      );
      const sortedTargets = [...targetStats.entries()].sort((a, b) =>
        b[1].broken - a[1].broken
        || b[1].decrypting - a[1].decrypting
        || a[1].country.name.localeCompare(b[1].country.name),
      );

      const columns: MeterMatrixColumn[] = sortedTargets.map(([id, info]) => ({
        key: String(id),
        flag: info.country.flag,
        name: info.country.name,
      }));

      const rows: MeterMatrixRow[] = sortedOwners.map(([ownerId, ownerInfo]) => {
        const cells: MeterMatrixCell[] = sortedTargets.map(([targetId, targetInfo]) => {
          const cellKey = `${ownerId}:${targetId}`;
          if (ownerId === targetId) {
            return {
              key: cellKey,
              kind: 'diagonal',
              spoken: `${ownerInfo.country.name} diagonal`,
            };
          }
          const pair = pairMap.get(cellKey);
          if (!pair) {
            return {
              key: cellKey,
              kind: 'none',
              spoken: `${ownerInfo.country.name} to ${targetInfo.country.name}, no attempt`,
            };
          }
          const numbers = readCipherNumbers(live, pair);
          const percent = percentOf(pair, numbers);
          const tooltipItems = pairTooltipItems(pair, reply, numbers, ownerInfo.country, targetInfo.country, percent);
          const title = `${ownerInfo.country.name} \u2192 ${targetInfo.country.name}`;
          if (pair.broken === true) {
            const brokenDate = pair.brokenAt ? formatDateOnly(pair.brokenAt.hour) : 'Date not recorded';
            return {
              key: cellKey,
              kind: 'broken',
              color: ownerInfo.country.colour,
              spoken: `${ownerInfo.country.name} reads ${targetInfo.country.name}, cipher broken ${brokenDate}`,
              audit: `broken:${brokenDate}`,
              tooltip: { title, hour: readView.hour, items: tooltipItems },
            };
          }
          if (percent === null) {
            return {
              key: cellKey,
              kind: 'missing',
              spoken: `${ownerInfo.country.name} to ${targetInfo.country.name}, progress not recorded`,
              audit: 'not-recorded',
              tooltip: { title, hour: readView.hour, items: tooltipItems },
            };
          }
          return {
            key: cellKey,
            kind: 'progress',
            fraction: percent / 100,
            color: ownerInfo.country.colour,
            spoken: `${ownerInfo.country.name} decrypting ${targetInfo.country.name}, ${percent}%`,
            audit: `${percent}%`,
            tooltip: { title, hour: readView.hour, items: tooltipItems },
          };
        });

        return {
          key: String(ownerId),
          ownerFlag: ownerInfo.country.flag,
          ownerName: ownerInfo.country.name,
          totalText: `Broken ${ownerInfo.broken} \u00b7 Decrypting ${ownerInfo.decrypting}`,
          cells,
        };
      });

      return {
        hour: readView.hour,
        layout: 'matrix',
        summary,
        matrix: {
          cornerLabel: 'Reads',
          columns,
          rows,
        },
        notes,
        warnings: reply.warnings,
      };
    }

    const sections: MeterSection[] = [];
    if (activePairs.length) {
      const rows: MeterRow[] = activePairs.map(pair => {
        const owner = countryOf(pair.owner);
        const target = countryOf(pair.target);
        const numbers = readCipherNumbers(live, pair);
        const percent = percentOf(pair, numbers);
        const key = `active:${pairKey(pair)}`;
        const title = `${owner.name} \u2192 ${target.name}`;
        return {
          key,
          name: title,
          pair: {
            ownerFlag: owner.flag,
            ownerName: owner.name,
            targetFlag: target.flag,
            targetName: target.name,
          },
          bar: {
            kind: 'track',
            fraction: percent === null ? null : percent / 100,
            color: owner.colour,
            height: 8,
          },
          figure: percent === null ? undefined : `${percent}%`,
          figureMissing: percent === null,
          spoken: `${owner.name} decrypting ${target.name}, ${progressLabel(percent)}, ${workingStatus(pair)}`,
          audit: `${owner.tag}>${target.tag}|${percent === null ? 'not-recorded' : `${percent}%`}`,
          tooltip: {
            title,
            hour: readView.hour,
            items: pairTooltipItems(pair, reply, numbers, owner, target, percent),
          },
        };
      });
      sections.push({
        key: 'decrypting',
        title: 'Decrypting',
        count: String(activePairs.length),
        rows,
      });
    }

    if (brokenPairs.length) {
      const grouped = new Map<number, { owner: ResolvedCountry; items: CipherPair[] }>();
      for (const pair of brokenPairs) {
        const group = grouped.get(pair.owner.id) ?? { owner: countryOf(pair.owner), items: [] };
        group.items.push(pair);
        grouped.set(pair.owner.id, group);
      }
      const lines = [...grouped.values()].sort((a, b) =>
        b.items.length - a.items.length || a.owner.name.localeCompare(b.owner.name),
      );
      const chipLines: MeterChipLine[] = lines.map(group => {
        const sortedItems = [...group.items].sort((a, b) =>
          (b.brokenAt?.hour ?? -Infinity) - (a.brokenAt?.hour ?? -Infinity)
          || countryOf(a.target).name.localeCompare(countryOf(b.target).name),
        );
        const chips: MeterChip[] = sortedItems.map(pair => {
          const target = countryOf(pair.target);
          const numbers = readCipherNumbers(live, pair);
          const percent = percentOf(pair, numbers);
          const dateText = pair.brokenAt ? formatDateOnly(pair.brokenAt.hour) : null;
          const title = `${group.owner.name} \u2192 ${target.name}`;
          return {
            key: `broken:${pairKey(pair)}`,
            targetFlag: target.flag,
            targetName: target.name,
            dateText,
            spoken: `${group.owner.name} broke ${target.name} cipher${dateText ? ` on ${dateText}` : ', date not recorded'}`,
            audit: `${group.owner.tag}>${target.tag}|broken|${dateText ?? 'not-recorded'}`,
            tooltip: {
              title,
              hour: readView.hour,
              items: pairTooltipItems(pair, reply, numbers, group.owner, target, percent),
            },
          };
        });
        return {
          key: `owner:${group.owner.key}`,
          ownerFlag: group.owner.flag,
          ownerName: group.owner.name,
          chips,
        };
      });
      sections.push({
        key: 'broken',
        title: 'Cipher broken',
        count: String(brokenPairs.length),
        chipLines,
      });
    }

    return {
      hour: readView.hour,
      layout: 'ciphers',
      summary,
      sections,
      notes,
      warnings: reply.warnings,
    };
  },
});

export const decryptionDeclaration: MeterWidgetDeclaration<DecryptionProjectionOptions> = {
  family: 'meters',
  kind: 'decryption',
  label: 'Decryption',
  subject: 'decryption',
  emptyCampaign: 'Load a campaign to see its decryption races.',
  icon: 'lock',
  optionFields: OPTIONS,
  countries: { mode: 'filter', labels: true, colours: true },
  sources: ctx => ({
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
  }),
  projection: decryptionProjection,
  projectionOptions: ctx => ({
    view: ctx.options.view === 'matrix' ? 'matrix' : 'ciphers',
    filter: ctx.countries.filter,
    filterAll: ctx.countries.all,
    side: ctx.options.countrySide === 'target' ? 'target' : 'decryptor',
  }),
  jumpToDate: true,
  keyByBranch: true,
};
