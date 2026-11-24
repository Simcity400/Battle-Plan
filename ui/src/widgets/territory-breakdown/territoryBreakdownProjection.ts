import { usNumber } from '../../charts/numberFormat.ts';
import { AREA_UNIT, type TerritoryMeasure } from '../../dashboard/territoryMeasure.ts';
import type { CountryGroupReply } from '../../generated/dashboard';
import { flagArtHandle } from '../../families/art/provider.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { BreakdownModel, BreakdownProjection } from '../../families/breakdown/model.ts';
import { registerBreakdownProjection } from '../../families/breakdown/registry.ts';
import { buildBreakdownTree, type RawBreakdownLeaf } from '../../families/breakdown/tree.ts';
import { affiliation, rowAmount, type RegionalRule } from './breakdownData.ts';
import type { TerritoryBreakdownReply } from './types';

export interface TerritoryBreakdownProjectionOptions {
  readonly measure: TerritoryMeasure;
  readonly rule: RegionalRule;
  readonly levels: readonly boolean[];
  readonly selectedTags: readonly string[] | null;
  readonly groups: CountryGroupReply | null;
  readonly factionColours: Readonly<Record<string, string>>;
  readonly factionNames: Readonly<Record<string, string>>;
  readonly world: boolean;
}

function formatTerritoryExact(value: number, exact: bigint | null, denominator: bigint): string {
  if (exact == null) return usNumber.format(Math.round(value));
  const whole = exact / denominator;
  const remainder = exact % denominator;
  return remainder === 0n
    ? usNumber.format(whole)
    : `${whole === 0n ? '' : `${usNumber.format(whole)} `}${remainder}/${denominator}`;
}

export const territoryBreakdownProjection: BreakdownProjection<TerritoryBreakdownProjectionOptions> = {
  kind: 'territory-breakdown',
  project(readView: BoundedReadView, options: TerritoryBreakdownProjectionOptions): BreakdownModel {
    const reply = readView.panel<TerritoryBreakdownReply>('breakdown');
    const live = readView.live('breakdown');
    const hour = Math.floor(readView.hour);
    const rootLabel = options.world ? 'World territory' : 'Selected territory';
    const unit = options.measure === 'area' ? AREA_UNIT : options.measure === 'victoryPoints' ? 'victory points' : options.measure;
    const verb = options.rule === 'owned' ? 'Owned' : 'Controlled';

    if (!reply || !live) {
      return {
        hour,
        rootLabel,
        total: { known: false, value: null, text: '0', unit, verb },
        parts: [],
        empty: 'No territory at this hour.',
      };
    }

    if (reply.missing) {
      return {
        hour,
        rootLabel,
        total: { known: false, value: null, text: 'Not recorded', unit, verb },
        parts: [],
        empty: reply.missing,
        emptyKind: 'unavailable',
      };
    }

    if (options.measure === 'area' && reply.areaReason) {
      return {
        hour,
        rootLabel,
        total: { known: false, value: null, text: 'Not recorded', unit, verb },
        parts: [],
        empty: reply.areaReason,
        emptyKind: 'unavailable',
      };
    }

    const regions = new Map(reply.regions.map(r => [r.id, r]));
    const factionOf: ReturnType<typeof affiliation> = options.groups
      ? affiliation(options.groups, hour)
      : () => ({ unknown: false });
    const selectedSet = options.selectedTags ? new Set(options.selectedTags) : null;
    const leaves: RawBreakdownLeaf[] = [];

    for (const row of reply.rows) {
      if (selectedSet && (row.tag === null || !selectedSet.has(row.tag))) continue;
      const amount = rowAmount(live as never, row.key, options.measure);
      if (amount && amount.exact === 0n) continue;

      const held = row.slot !== null;
      const member = row.tag !== null ? factionOf(row.tag) : { unknown: false };
      const who = row.tag !== null ? readView.who('country', row.tag, hour) : null;
      const countryName = who?.name.status === 'ready' ? who.name.value : (row.tag ?? (held ? 'Country not named at this hour' : 'Unassigned territory'));
      const countryColor = who?.colour?.status === 'ready' ? who.colour.value : undefined;
      const flagArt = who?.flagChoice?.status === 'ready' ? readView.art(flagArtHandle(who.flagChoice.value, 'panel')) : undefined;

      let factionKey: string;
      let factionLabel: string;
      let factionColor: string | undefined;
      let factionNeutral = false;
      if (member.unknown) {
        factionKey = 'faction-unknown';
        factionLabel = 'Faction not recorded';
        factionNeutral = true;
      } else if (member.faction) {
        factionKey = `faction:${member.faction.id}`;
        const fWho = readView.who('faction', member.faction.id, hour);
        factionLabel = fWho.name.status === 'ready'
          ? fWho.name.value
          : (options.factionNames[member.faction.id] ?? member.faction.names.find(n => n.firstHour <= hour && hour <= n.lastHour)?.name ?? 'Unnamed faction');
        factionColor = fWho.colour?.status === 'ready' ? fWho.colour.value : options.factionColours[member.faction.id];
      } else if (held) {
        factionKey = 'non-aligned';
        factionLabel = 'Non-aligned';
        factionNeutral = true;
      } else {
        factionKey = 'unassigned-territory';
        factionLabel = 'Unassigned territory';
        factionNeutral = true;
      }

      const region = row.region === null ? null : regions.get(row.region);
      const regionKey = row.region === null ? 'region-unattributed' : String(row.region);
      const regionName = region?.name ?? (row.region === null ? 'Unknown region' : `Region ${row.region}`);

      leaves.push({
        path: [
          { key: factionKey, name: factionLabel, color: factionColor, neutral: factionNeutral },
          {
            key: row.tag !== null ? `country:${row.tag}` : held ? `slot:${row.slot}` : 'unassigned',
            name: countryName,
            color: countryColor,
            flag: row.tag ?? undefined,
            flagArt,
          },
          { key: regionKey, name: regionName },
        ],
        value: amount ? amount.value : null,
        exact: amount ? amount.exact : null,
        exactDenominator: amount ? amount.denominator : 1n,
        notRecorded: amount === null,
        missingReason: amount === null ? 'Territory amount not recorded at this hour.' : undefined,
        partial: member.unknown,
        partialReason: member.unknown ? 'Faction membership was not recorded at this hour.' : undefined,
      });
    }

    const periodLabel = `${verb} on ${formatDateOnly(hour)}`;
    if (!leaves.length) {
      return {
        hour,
        rootLabel,
        periodLabel,
        total: { known: true, value: 0, text: '0', unit, verb },
        parts: [],
        empty: 'No territory at this hour.',
      };
    }

    const { parts, total } = buildBreakdownTree({
      leaves,
      levels: options.levels,
      formatValue: formatTerritoryExact,
      unit,
      verb,
    });

    return {
      hour,
      rootLabel,
      periodLabel,
      total,
      parts,
      warnings: reply.warnings,
    };
  },
};

registerBreakdownProjection(territoryBreakdownProjection);
