import { flagArtHandle, flagChoiceArt } from '../../families/art/provider.ts';
import { artHandleKey } from '../../families/art/types.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import type { TableCell, TableColumnSpec, TableModel, TablePane, TablePaneTitle, TableProjection, TableRow, TooltipExtraRow } from '../../families/table/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { knownValue, notApplicable, notRecorded, partialValue, zeroValue, type Value } from '../../families/value.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { WarMeasure, WarMeasureKey, WarMeasureMember, WarMeasureSide, WarsReply } from '../../generated/dashboard';
import { chosenGroup, contributionPartial, leadOf, savedWar, sideCountries, warContributionNextChange, warContributionSources, warCountries, warsAux, type ContributionGroup, type WarChoiceOptions, type WarSide } from './warContributionPlan.ts';
import { resolveName } from '../../families/table/identityCell.ts';

export const SIDES_VIEW = 'sides';

const MEASURES: readonly { readonly key: WarMeasureKey; readonly label: string; readonly hint: string }[] = [
  { key: 'participation', label: 'Participation', hint: 'War participation' },
  { key: 'equipmentDamage', label: 'Equipment', hint: 'Equipment destroyed' },
  { key: 'occupation', label: 'Territory', hint: 'Territory occupied' },
  { key: 'airDamage', label: 'Air', hint: 'Air damage' },
  { key: 'shipDamage', label: 'Naval', hint: 'Naval damage' },
  { key: 'casualties', label: 'Casualties', hint: 'Casualties inflicted' },
  { key: 'strategicBombing', label: 'Bombing', hint: 'Strategic bombing' },
  { key: 'convoyDamage', label: 'Convoys', hint: 'Convoy raiding' },
  { key: 'lendLeaseSent', label: 'Lend-lease out', hint: 'Lend-lease sent' },
  { key: 'lendLeaseReceived', label: 'Lend-lease in', hint: 'Lend-lease received' },
];

const NOT_A_MEMBER = 'This country has no recorded part in this measure';
const PARTICIPATION_IS_A_SHARE = 'Participation is already each country’s share of its side';
const NO_SHARE = 'No part of this measure was recorded for either side';
const RAW_AMOUNT = { decimals: 5 } as const;

export type SidesOptions = WarChoiceOptions;

function columnsOf(measures: readonly WarMeasure[] | null): TableColumnSpec[] {
  const present = measures ? new Set(measures.map(measure => measure.key)) : null;
  return [
    { key: 'country', label: 'Country', as: 'flagName', offered: ['flagName'], priority: 0, text: true, pinned: true, sortable: true, hideable: false, minWidth: 150, maxWidth: 186 },
    ...MEASURES.filter(measure => !present || present.has(measure.key)).map((measure, at): TableColumnSpec => ({
      key: measure.key,
      label: measure.label,
      hint: measure.hint,
      as: 'plain',
      offered: ['plain'],
      priority: at + 1,
      numeric: true,
      cellKind: 'share',
      barEligible: false,
      minWidth: 96,
    })),
  ];
}

const ZERO = { decimals: 0 } as const;

function shareValue(permyriad: number | null, raw: string | null, partial: string, missing: string): Value<number> {
  if (permyriad === null) return notRecorded(missing);
  const share = permyriad / 10000;
  if (partial) return partialValue(share, partial);
  return raw === '0' ? zeroValue() : knownValue(share);
}

function memberCell(member: WarMeasureMember | undefined, colour: string | undefined, missing: string, partial: string | undefined): TableCell {
  const value = member ? shareValue(member.shareOfSide, member.value, contributionPartial(partial, member.partial), NO_SHARE) : notRecorded(missing);
  return {
    value,
    format: value.kind === 'zero' ? ZERO : 'share',
    bar: { fraction: member && member.shareOfSide !== null ? knownValue(member.shareOfSide / 10000) : value },
    ...(colour ? { countryColor: colour } : {}),
    ...(member?.shareOfSide != null ? { sortValue: knownValue(member.shareOfSide) } : {}),
  };
}

function totalCell(side: WarMeasureSide, measure: WarMeasureKey, missing: string, partial: string | undefined): TableCell {
  if (measure === 'participation') return { value: notApplicable(PARTICIPATION_IS_A_SHARE), bar: null };
  const value = shareValue(side.shareOfWar, side.total, contributionPartial(partial, side.partial), missing);
  return { value, format: value.kind === 'zero' ? ZERO : 'share', bar: null };
}

interface Who {
  readonly cell: TableCell;
  readonly title: TablePaneTitle;
  readonly name: string;
}

function who(readView: BoundedReadView, tag: string, hour: number): Who {
  const { value: name, name: shownName, who: identity } = resolveName(readView, 'country', tag, { atHour: hour });
  const handle = identity.flagChoice?.status === 'ready' ? flagArtHandle(identity.flagChoice.value, 'small') : null;
  const flagArt = handle ? readView.art(handle) : flagChoiceArt(readView, identity.flagChoice, 'small');
  const artKey = handle ? artHandleKey(handle) : undefined;
  const flag = flagArt.status === 'ready' ? flagArt.url : undefined;
  const flagPending = flagArt.status === 'pending';
  const colour = identity.colour?.status === 'ready' ? identity.colour.value : undefined;
  return {
    cell: { value: name, iconSlot: true, country: tag, art: flagArt, ...(artKey ? { artKey } : {}), ...(colour ? { countryColor: colour } : {}) },
    title: { name, country: tag, ...(flag ? { flag } : {}), ...(flagPending ? { flagPending: true } : {}) },
    name: shownName,
  };
}

function paneOf(readView: BoundedReadView, group: ContributionGroup, side: WarSide, lead: string, hour: number): TablePane {
  const { measures, partial } = group;
  const participation = measures.find(measure => measure.key === 'participation') ?? measures[0];
  const order = participation && participation[side].total !== null ? participation[side].members.map(member => member.tag) : sideCountries(group, side);
  const missing = group.reason ?? NOT_A_MEMBER;
  const byMeasure = new Map(measures.map(measure => [measure.key, new Map(measure[side].members.map(member => [member.tag, member]))]));
  const rows: TableRow[] = order.map(tag => {
    const country = who(readView, tag, hour);
    const colour = country.cell.countryColor;
    return {
      key: `${side}:${tag}`,
      name: country.name,
      group: { key: 'side', label: 'Share of the side' },
      cells: {
        country: country.cell,
        ...Object.fromEntries(measures.map(measure => [measure.key, memberCell(byMeasure.get(measure.key)!.get(tag), colour, missing, partial)])),
      },
      tooltipTitle: country.name,
      tooltipExtra: (): TooltipExtraRow[] => MEASURES.flatMap(measure => {
        const members = byMeasure.get(measure.key);
        if (!members) return [];
        const member = members.get(tag);
        const reason = member && contributionPartial(partial, member.partial);
        const value = !member ? notRecorded(missing) : reason ? partialValue(BigInt(member.value), reason) : knownValue(BigInt(member.value));
        return [{ label: measure.hint, value, format: RAW_AMOUNT }];
      }),
    };
  });
  const totalLabel = partial ? 'Share within the recorded group' : 'Share of the war';
  return {
    key: side,
    title: who(readView, lead, hour).title,
    rows,
    totalRow: {
      key: `total:${side}`,
      name: totalLabel,
      cells: {
        country: { value: knownValue(totalLabel) },
        ...Object.fromEntries(measures.map(measure => [measure.key, totalCell(measure[side], measure.key, group.reason ?? NO_SHARE, partial)])),
      },
    },
  };
}

export const warContributionSidesProjection: TableProjection<SidesOptions> = registerFamilyProjection({
  kind: 'war-contribution',
  family: 'table',
  nextChange(readView, options) {
    return warContributionNextChange(readView.hour, options);
  },
  project(readView, options): TableModel {
    const hour = readView.hour;
    const chosen = chosenGroup(readView, options);
    if (chosen.kind === 'loading') return { hour, columns: columnsOf(null), rows: [] };
    if (chosen.kind === 'blank') return { hour, columns: columnsOf(null), rows: [], empty: chosen.empty, emptyKind: chosen.emptyKind };
    const { group, reply, asOf, sides: [left, right] } = chosen;
    const heldFromEarlier = asOf !== hour;
    return {
      hour,
      ...(heldFromEarlier ? { tooltipHour: asOf, titleSummary: `As of ${formatDateOnly(asOf)}` } : {}),
      columns: columnsOf(group.measures),
      rows: [],
      panes: [paneOf(readView, group, left, leadOf(group, left), asOf), paneOf(readView, group, right, leadOf(group, right), asOf)],
      ...(reply.warnings.length ? { warnings: reply.warnings } : {}),
    };
  },
}, SIDES_VIEW);

export const warContributionSidesDeclaration: TableWidgetDeclaration<SidesOptions, WarsReply> = {
  family: 'table',
  kind: 'war-contribution',
  label: 'War contribution',
  subject: 'war contributions',
  emptyCampaign: 'Load a campaign to see war contributions.',
  icon: 'swords',
  countries: warCountries,
  aux: warsAux,
  sources: warContributionSources,
  projection: warContributionSidesProjection,
  columns: () => columnsOf(null),
  settings: { kind: 'table', base: { density: 'compact', barsBehind: 'off', sort: { column: 'participation', direction: 'descending' } } },
  projectionOptions: ctx => ({ wars: ctx.aux ?? null, saved: savedWar(ctx.options) }),
};
