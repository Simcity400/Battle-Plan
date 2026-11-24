import { liveKey } from '../../charts/live.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { gameWindow } from '../../dashboard/gameWindows.ts';
import { object } from '../../dashboard/validation.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { urlArtHandle } from '../../families/art/provider.ts';
import { artKindCell } from '../../families/table/identityCell.ts';
import type { TableColumnSpec, TableModel, TableProjection, TableRow, TooltipExtraRow } from '../../families/table/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded, valueFromLive, zeroValue, type Value } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { IndustrialOrganisationsReply, OrganisationNow } from '../../generated/dashboard';
import { COLUMNS, TABLE_BASE } from './definition.ts';

export interface OrganisationsProjectionOptions {
  readonly canOpen: boolean;
}

export interface OrganisationMeta {
  readonly organisation: string;
  readonly name: string;
}

const TEXT = (value: unknown) => value === null || typeof value === 'string';
const COUNT = (value: unknown) => value === null || Number.isInteger(value);
const FIXED5 = 100000n;
const HOURS_A_DAY = 24;

export function validateOrganisations(value: unknown): IndustrialOrganisationsReply {
  const reply = value as IndustrialOrganisationsReply;
  if (!object(reply) || typeof reply.tag !== 'string' || !TEXT(reply.missing) || !Array.isArray(reply.warnings)
    || !Array.isArray(reply.icons) || !reply.icons.every(icon => object(icon) && typeof icon.key === 'string' && typeof icon.image === 'string')
    || !Array.isArray(reply.organisations) || !reply.organisations.every(row => object(row) && /^\d{1,20}$/.test(row.id) && typeof row.token === 'string'
      && [row.name, row.nameUnknown, row.icon, row.iconUnknown, row.policyToken, row.policyName, row.policyUnknown, row.detailsMissing].every(TEXT)
      && [row.size, row.traits, row.cooldownHour].every(COUNT))) {
    throw new Error('Invalid industrial organisations reply');
  }
  return reply;
}

const PRIORITY: Readonly<Record<string, number>> = { kind: 0, organisation: 0, size: 1, funds: 2, policy: 3, traits: 4, view: 5 };
export const ORGANISATION_COLUMNS: readonly TableColumnSpec[] = COLUMNS.map(column => ({
  ...column,
  priority: PRIORITY[column.key] ?? 9,
  numeric: column.key === 'size' || column.key === 'funds' || column.key === 'traits',
  text: column.key === 'organisation' || column.key === 'policy',
  ...(column.key === 'kind' ? { cellKind: 'kind' as const, sortable: false, pinned: true } : {}),
  ...(column.key === 'organisation' ? { cellKind: 'item' as const, pinned: true } : {}),
  ...(column.key === 'size' || column.key === 'funds' || column.key === 'traits' ? { barEligible: true } : {}),
  ...(column.key === 'view' ? { cellKind: 'action' as const, sortable: false } : {}),
}));

const count = (value: number | null, missing: string): Value => value === null ? notRecorded(missing) : value === 0 ? zeroValue(0) : knownValue(value);

export function cooldownDays(cooldownHour: number | null, hour: number): number | null {
  if (cooldownHour === null) return null;
  const days = Math.floor(cooldownHour / HOURS_A_DAY) - Math.floor(hour / HOURS_A_DAY);
  return days > 0 ? days : null;
}

function policyValue(row: OrganisationNow): Value<string> {
  if (row.policyName) return knownValue(row.policyName);
  if (row.policyToken !== null || row.policyUnknown !== null) return notRecorded(row.policyUnknown ?? 'Policy name not recorded');
  return row.size === null ? notRecorded(row.detailsMissing ?? 'Policy not recorded') : knownValue('None');
}

function funds(value: Value): Value {
  if (value.kind !== 'known') return value;
  const units = BigInt(Math.trunc(Number(value.value))) / FIXED5;
  return units === 0n ? zeroValue(0n) : knownValue(units);
}

export const organisationsProjection: TableProjection<OrganisationsProjectionOptions> = registerFamilyProjection({
  kind: 'industrial-organisations',
  family: 'table',
  project(readView, options): TableModel {
    const hour = readView.hour;
    const refused = readView.refused('main');
    if (refused !== undefined) return { columns: ORGANISATION_COLUMNS, rows: [], hour, refused };
    const raw = readView.panel('main');
    if (raw === undefined) return { columns: ORGANISATION_COLUMNS, rows: [], hour };
    const reply = validateOrganisations(raw);
    if (reply.missing !== null) {
      return { columns: ORGANISATION_COLUMNS, rows: [], hour, empty: 'Not recorded', emptyDetail: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
    }
    const live = readView.live('main') ?? null;
    const images = new Map(reply.icons.map(icon => [icon.key, icon.image]));
    let nextDay = Infinity;
    const rows: TableRow[] = reply.organisations.map(row => {
      const image = row.icon ? images.get(row.icon) ?? null : null;
      const name = row.name ?? row.token;
      const days = cooldownDays(row.cooldownHour, hour);
      if (days !== null) nextDay = Math.min(nextDay, (Math.floor(hour / HOURS_A_DAY) + 1) * HOURS_A_DAY);
      const meta: OrganisationMeta = { organisation: row.id, name };
      const extra = (): readonly TooltipExtraRow[] => [
        { label: 'Unspent points', value: valueFromLive(live, liveKey(row.id, 'points')), format: 'integer', divider: true },
        ...(days !== null ? [{ label: 'Policy cooldown', value: knownValue(`${formatScalar(days, 'integer')} days`) }] : []),
      ];
      return {
        key: row.id,
        name,
        cells: {
          kind: artKindCell(readView, urlArtHandle(image), 'wide', row.name),
          organisation: { value: row.name ? knownValue(row.name) : notRecorded(row.nameUnknown ?? 'Organisation name not recorded') },
          size: { value: count(row.size, row.detailsMissing ?? 'Size not recorded'), format: 'integer' },
          funds: { value: funds(valueFromLive(live, liveKey(row.id, 'funds'), row.detailsMissing ? { overrideMissing: row.detailsMissing } : undefined)), format: 'integer' },
          traits: { value: count(row.traits, row.detailsMissing ?? 'Traits not recorded'), format: 'integer' },
          policy: { value: policyValue(row) },
          view: { value: options.canOpen ? knownValue('View') : notApplicable('The policy window is not available yet') },
        },
        tooltipExtra: extra,
        ...(options.canOpen ? { tooltipAction: 'Open its policies' } : {}),
        meta,
      };
    });
    return {
      columns: ORGANISATION_COLUMNS,
      rows,
      hour,
      nextChangeAt: nextDay,
      rowTimeBoundaries: false,
      ...(rows.length === 0 ? { empty: 'No industrial organisations', emptyKind: 'empty' as const } : {}),
      warnings: reply.warnings,
    };
  },
});

export const organisationsDeclaration: TableWidgetDeclaration<OrganisationsProjectionOptions> = {
  family: 'table',
  kind: 'industrial-organisations',
  label: 'Industrial organisations',
  subject: 'organisations',
  emptyCampaign: 'Load a campaign to see organisations.',
  icon: 'factory',
  countries: { mode: 'single', choose: 'Choose a country to see its organisations.' },
  sources: ctx => {
    const params = ctx.countryTag ? { branchId: ctx.branch.id, country: countryFromKey(ctx.countryTag) } : null;
    return {
      owner: `industrial-organisations:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${ctx.countryTag ?? ''}`,
      entries: params
        ? [{
            name: 'main',
            source: ctx.track.source,
            branch: ctx.branch.id,
            offset: ctx.track.axisOffset,
            read: {
              shape: 'point',
              channel: 'panel',
              coverage: { mode: 'keyed', keys: [ctx.countryTag!] },
              product: 'industrial-organisations',
              followQuery: { panel: 'industrial-organisations', ...params },
              params,
              validate: validateOrganisations,
            },
          }]
        : [],
    };
  },
  projection: organisationsProjection,
  columns: ORGANISATION_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: TABLE_BASE },
  projectionOptions: () => ({ canOpen: gameWindow('industrialOrganisation') !== null }),
};
