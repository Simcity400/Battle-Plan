import { gameWindow } from '../../dashboard/gameWindows.ts';
import type { WidgetSpec } from '../../dashboard/model';
import type { BoundedReadView } from '../../families/contract.ts';
import type { DeclaredCountryFilter, TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { identityCell, identityName, newIdentityBoundary } from '../../families/table/identityCell.ts';
import type { TableColumnSpec, TableModel, TableProjection, TableRow, TooltipExtraRow } from '../../families/table/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded, zeroValue, type Value } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { Agency, AgencyComparisonReply, Upgrades } from '../../generated/dashboard';
import { COLUMNS, TABLE_BASE } from './definition.ts';

export interface AgencyProjectionOptions {
  readonly filter: DeclaredCountryFilter;
  readonly topped: boolean;
  readonly canOpen: boolean;
}

export interface AgencyMeta {
  readonly tag: string;
  readonly name: string;
}

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const integer = (value: unknown): value is number => Number.isSafeInteger(value);
const text = (value: unknown): value is string | null => value === null || typeof value === 'string';
const raw = (value: unknown): value is string => typeof value === 'string' && /^-?\d{1,20}$/.test(value);
const country = (value: unknown) => object(value) && integer(value.id) && (value.tag === null || (typeof value.tag === 'string' && value.tag.length > 0));
const upgradesOk = (value: unknown): value is Upgrades => object(value) && text(value.unavailable)
  && (value.nonnegativeLevelSum === null || (typeof value.nonnegativeLevelSum === 'string' && /^(0|[1-9]\d*)$/.test(value.nonnegativeLevelSum)))
  && (value.unavailable === null) === (value.nonnegativeLevelSum !== null)
  && Array.isArray(value.items) && value.items.every(item => object(item) && integer(item.ordinal) && text(item.branch) && text(item.token)
    && integer(item.level) && text(item.name) && text(item.missing));

export function validateAgencies(value: unknown): AgencyComparisonReply {
  if (!object(value) || !Array.isArray(value.agencies) || !value.agencies.every(item => object(item) && country(item.country) && typeof item.created === 'boolean'
    && text(item.name) && text(item.nameMissing) && raw(item.defenseRaw) && text(item.defenseText) && text(item.defenseValue) && integer(item.activeCount)
    && integer(item.maximum) && (item.upgrades === null || upgradesOk(item.upgrades))) || !text(value.defenseLabel) || !text(value.upgradesMissing)
    || !text(value.missing) || !Array.isArray(value.warnings) || !value.warnings.every(item => typeof item === 'string')) {
    throw new Error('The agency comparison reply is malformed.');
  }
  return value as unknown as AgencyComparisonReply;
}

const PRIORITY: Readonly<Record<string, number>> = { agency: 0, defense: 1, country: 2, operatives: 3, upgrades: 4, view: 5 };
const FIXED5_DECIMALS = { decimals: 5 } as const;
export const AGENCY_COLUMNS: readonly TableColumnSpec[] = COLUMNS.map(column => ({
  ...column,
  priority: PRIORITY[column.key] ?? 9,
  numeric: column.key === 'upgrades' || column.key === 'defense' || column.key === 'operatives',
  text: column.key === 'country' || column.key === 'agency',
  ...(column.key === 'country' ? { pinned: true } : {}),
  ...(column.key === 'agency' ? { cellKind: 'item' as const, pinned: true } : {}),
  ...(column.key === 'upgrades' ? { barEligible: true } : {}),
  ...(column.key === 'defense' ? { format: FIXED5_DECIMALS, barEligible: false, widest: ['99.99'] } : {}),
  ...(column.key === 'operatives' ? { cellKind: 'outOf' as const, barEligible: false, unit: 'active / maximum', format: 'integer' as const, widest: ['99 / 99'] } : {}),
  ...(column.key === 'view' ? { cellKind: 'action' as const, sortable: false } : {}),
}));

const passes = (filter: DeclaredCountryFilter, tag: string | null, hour: number): boolean =>
  filter.passing === null || (tag !== null && !!filter.passing[tag]?.some(([first, last]) => first <= hour && hour <= last));

function filterNext(filter: DeclaredCountryFilter, hour: number): number {
  for (const bound of filter.bounds) if (bound > hour) return bound;
  return Infinity;
}

function upgradesValue(agency: Agency, missing: string | null): Value {
  const held = agency.upgrades;
  if (!held) return notRecorded(missing ?? 'Upgrades not recorded');
  if (held.nonnegativeLevelSum === null) return notRecorded(held.unavailable ?? 'Upgrades not recorded');
  const sum = BigInt(held.nonnegativeLevelSum);
  return sum === 0n ? zeroValue(0n) : knownValue(sum);
}

function defenseCell(agency: Agency): { readonly value: Value; readonly sortValue?: Value } {
  const raw = BigInt(agency.defenseRaw);
  const exact = raw === 0n ? zeroValue(0n) : knownValue(raw);
  return agency.defenseValue ? { value: knownValue(agency.defenseValue), sortValue: exact } : { value: exact };
}

function upgradeRows(agency: Agency): readonly TooltipExtraRow[] {
  return (agency.upgrades?.items ?? []).map((item, index) => ({
    label: item.name ?? 'Upgrade',
    value: item.name ? knownValue(`Level ${item.level}`) : notRecorded(item.missing ?? 'Upgrade name not recorded'),
    ...(index === 0 ? { divider: true } : {}),
  }));
}

export const agencyProjection: TableProjection<AgencyProjectionOptions> = registerFamilyProjection({
  kind: 'agency-comparison',
  family: 'table',
  nextChange: (readView: BoundedReadView, options: AgencyProjectionOptions) => (options.topped ? Infinity : filterNext(options.filter, readView.hour)),
  project(readView, options): TableModel {
    const hour = readView.hour;
    const refused = readView.refused('main');
    if (refused !== undefined) return { columns: AGENCY_COLUMNS, rows: [], hour, refused };
    const raw = readView.panel('main');
    if (raw === undefined) return { columns: AGENCY_COLUMNS, rows: [], hour };
    const reply = validateAgencies(raw);
    if (reply.missing !== null) {
      return { columns: AGENCY_COLUMNS, rows: [], hour, empty: 'Not recorded', emptyDetail: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
    }
    const columns = reply.defenseLabel ? AGENCY_COLUMNS.map(column => (column.key === 'defense' ? { ...column, label: reply.defenseLabel! } : column)) : AGENCY_COLUMNS;
    const boundary = newIdentityBoundary();
    const rows: TableRow[] = [];
    for (const agency of reply.agencies) {
      const tag = agency.country.tag;
      if (!options.topped && !passes(options.filter, tag, hour)) continue;
      const name: Value<string> = agency.name !== null ? knownValue(agency.name) : notRecorded(agency.nameMissing ?? 'Agency name not recorded');
      const meta: AgencyMeta | undefined = tag ? { tag, name: agency.name ?? identityName(readView, 'country', tag) } : undefined;
      rows.push({
        key: String(agency.country.id),
        name: agency.name ?? tag ?? String(agency.country.id),
        cells: {
          country: identityCell(readView, 'country', tag, { boundary }),
          agency: { value: name },
          upgrades: { value: upgradesValue(agency, reply.upgradesMissing), format: 'integer' },
          defense: defenseCell(agency),
          operatives: {
            value: agency.activeCount === 0 ? zeroValue(0) : knownValue(agency.activeCount),
            secondaryValue: knownValue(formatScalar(agency.maximum, 'integer')),
          },
          view: { value: options.canOpen && meta ? knownValue('View') : notApplicable('The agency window is not available yet') },
        },
        tooltipExtra: () => [
          ...(agency.created ? [] : [{ label: 'Agency', value: knownValue('Not created'), tone: 'muted' as const }]),
          ...upgradeRows(agency),
        ],
        ...(options.canOpen && meta ? { tooltipAction: 'Open its upgrades' } : {}),
        ...(meta ? { meta } : {}),
      });
    }
    return {
      columns,
      rows,
      hour,
      nextChangeAt: boundary.next,
      rowTimeBoundaries: false,
      ...(rows.length === 0 ? { empty: 'No agencies', emptyKind: 'empty' as const } : {}),
      warnings: reply.warnings,
    };
  },
});

function topOf(widget: WidgetSpec): number | null {
  const countries = widget.countries;
  return countries.kind === 'group' && countries.group.type === 'top' ? countries.group.count : null;
}

const TOP_MEASURES = new Set(['defense', 'active', 'maximum']);

export const agencyDeclaration: TableWidgetDeclaration<AgencyProjectionOptions> = {
  family: 'table',
  kind: 'agency-comparison',
  label: 'Agency comparison',
  subject: 'agencies',
  emptyCampaign: 'Load a campaign to see agencies.',
  icon: 'shield',
  countries: { mode: 'filter' },
  sources: ctx => {
    const top = topOf(ctx.widget);
    const by = typeof ctx.options.topMeasure === 'string' && TOP_MEASURES.has(ctx.options.topMeasure) ? ctx.options.topMeasure : 'defense';
    const params = {
      branchId: ctx.branch.id,
      includeUncreated: ctx.options.includeUncreated === true,
      ...(top !== null ? { top: { count: top, by } } : {}),
    };
    return {
      owner: `agency-comparison:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
      entries: [{
        name: 'main',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'point',
          channel: 'panel',
          coverage: top !== null ? { mode: 'population', form: 'top', limit: top } : { mode: 'population', form: 'all' },
          product: 'agency-comparison',
          followQuery: { panel: 'agency-comparison', ...params },
          params,
          validate: validateAgencies,
        },
      }],
    };
  },
  projection: agencyProjection,
  columns: AGENCY_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: TABLE_BASE },
  projectionOptions: ctx => ({ filter: ctx.countries.filter, topped: topOf(ctx.widget) !== null, canOpen: gameWindow('agencyUpgrades') !== null }),
};
