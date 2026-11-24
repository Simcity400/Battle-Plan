import { formatAmount } from '../../charts/valueAxis.ts';
import { flagArtHandle } from '../../families/art/provider.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import type { TableCell, TableColumnSpec, TableModel, TablePane, TablePaneTitle, TableProjection, TableRow, TooltipExtraRow } from '../../families/table/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { knownValue, notApplicable, notRecorded, partialValue, zeroValue, type Value } from '../../families/value.ts';
import type { WarContributionGroup, WarMeasure, WarMeasureKey, WarMeasureMember, WarMeasureSide, WarsReply } from '../../generated/dashboard';
import { chosenGroup, leadOf, savedWar, warContributionSources, warCountries, warsAux, type WarChoiceOptions, type WarSide } from './warContributionPlan.ts';

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

const UNATTRIBUTED = 'Part of this side’s casualties contribution is not attributed to a country';
const NOT_A_MEMBER = 'This country has no recorded part in this measure';
const PARTICIPATION_IS_A_SHARE = 'Participation is already each country’s share of its side';
const NO_SHARE = 'No part of this measure was recorded for either side';
const SCALE = 5;

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

function shareValue(permyriad: number | null, raw: string, partial: boolean, missing: string): Value<number> {
  if (permyriad === null) return notRecorded(missing);
  const share = permyriad / 10000;
  if (partial) return partialValue(share, UNATTRIBUTED);
  return raw === '0' ? zeroValue() : knownValue(share);
}

function memberCell(member: WarMeasureMember | undefined, colour: string | undefined): TableCell {
  const value = member ? shareValue(member.shareOfSide, member.value, member.partial, NO_SHARE) : notRecorded(NOT_A_MEMBER);
  return {
    value,
    format: value.kind === 'zero' ? ZERO : 'share',
    bar: { fraction: member && member.shareOfSide !== null ? knownValue(member.shareOfSide / 10000) : notRecorded(NOT_A_MEMBER) },
    ...(colour ? { countryColor: colour } : {}),
    ...(member?.shareOfSide != null ? { sortValue: knownValue(member.shareOfSide) } : {}),
  };
}

function totalCell(side: WarMeasureSide, measure: WarMeasureKey): TableCell {
  if (measure === 'participation') return { value: notApplicable(PARTICIPATION_IS_A_SHARE), bar: null };
  const value = shareValue(side.shareOfWar, side.total, side.partial, NO_SHARE);
  return { value, format: value.kind === 'zero' ? ZERO : 'share', bar: null };
}

interface Who {
  readonly cell: TableCell;
  readonly title: TablePaneTitle;
  readonly name: string;
}

function who(readView: BoundedReadView, tag: string, hour: number): Who {
  const identity = readView.who('country', tag, hour);
  const name: Value<string> = identity.name.status === 'ready'
    ? knownValue(identity.name.value)
    : identity.name.status === 'pending'
      ? knownValue(tag)
      : notRecorded(identity.name.status === 'absent' || identity.name.status === 'failed' ? identity.name.reason : 'Country name unavailable');
  const art = identity.flagChoice?.status === 'ready' ? readView.art(flagArtHandle(identity.flagChoice.value, 'small')) : null;
  const flag = art?.status === 'ready' ? art.url : undefined;
  const flagPending = identity.flagChoice === undefined || identity.flagChoice.status === 'pending' || art?.status === 'pending';
  const colour = identity.colour?.status === 'ready' ? identity.colour.value : undefined;
  return {
    cell: { value: name, iconSlot: true, country: tag, ...(flag ? { icon: flag } : {}), ...(flagPending ? { iconPending: true } : {}), ...(colour ? { countryColor: colour } : {}) },
    title: { name, country: tag, ...(flag ? { flag } : {}), ...(flagPending ? { flagPending: true } : {}) },
    name: name.kind === 'known' ? String(name.value) : tag,
  };
}

function amountRows(group: WarContributionGroup, tag: string): readonly TooltipExtraRow[] {
  const country = group.participants.find(participant => participant.tag === tag);
  if (!country) return [];
  const raw: Readonly<Record<WarMeasureKey, string>> = {
    participation: country.participation, equipmentDamage: country.equipmentDamage, occupation: country.occupation, airDamage: country.airDamage,
    shipDamage: country.shipDamage, casualties: country.casualties, strategicBombing: country.strategicBombing, convoyDamage: country.convoyDamage,
    lendLeaseSent: country.lendLeaseSent, lendLeaseReceived: country.lendLeaseReceived,
  };
  return MEASURES.map(measure => ({ label: measure.hint, value: knownValue(formatAmount(BigInt(raw[measure.key]), SCALE, 2)) }));
}

function paneOf(readView: BoundedReadView, group: WarContributionGroup, side: WarSide, lead: string, hour: number): TablePane {
  const measures = group.measures;
  const participation = measures.find(measure => measure.key === 'participation') ?? measures[0];
  const order = participation ? participation[side].members.map(member => member.tag) : (side === 'viewpoint' ? group.viewpointSide : group.opposingSide);
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
        ...Object.fromEntries(measures.map(measure => [measure.key, memberCell(byMeasure.get(measure.key)!.get(tag), colour)])),
      },
      tooltipTitle: country.name,
      tooltipExtra: () => amountRows(group, tag),
    };
  });
  return {
    key: side,
    title: who(readView, lead, hour).title,
    rows,
    totalRow: {
      key: `total:${side}`,
      name: 'Share of the war',
      cells: {
        country: { value: knownValue('Share of the war') },
        ...Object.fromEntries(measures.map(measure => [measure.key, totalCell(measure[side], measure.key)])),
      },
    },
  };
}

export const warContributionSidesProjection: TableProjection<SidesOptions> = registerFamilyProjection({
  kind: 'war-contribution',
  family: 'table',
  project(readView, options): TableModel {
    const hour = readView.hour;
    const chosen = chosenGroup(readView, options);
    if (chosen.kind === 'loading') return { hour, columns: columnsOf(null), rows: [] };
    if (chosen.kind === 'blank') return { hour, columns: columnsOf(null), rows: [], empty: chosen.empty, emptyKind: chosen.emptyKind };
    const { group, reply, sides: [left, right] } = chosen;
    return {
      hour,
      columns: columnsOf(group.measures),
      rows: [],
      panes: [paneOf(readView, group, left, leadOf(group, left), reply.hour), paneOf(readView, group, right, leadOf(group, right), reply.hour)],
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
