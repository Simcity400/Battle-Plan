import { liveKey } from '../../charts/live.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { declareTableProjection, type DeclarativeColumnSpec, type TableProjection } from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { OFFERED, TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, textValue, valueFromLive, zeroValue } from '../../families/value.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { AirMissionChoice, AirMissionRegion, AirMissionsReply } from '../../generated/dashboard';
import { validateAirMissions } from './airMissionsData.ts';
import { COLUMNS, TABLE_BASE, WINGS_COLUMN } from './definition.ts';

const NO_OPTIONS: Record<string, never> = {};

type MissionRow = { readonly region: AirMissionRegion; readonly flown: ReadonlySet<string> };

const REGION_COLUMN: DeclarativeColumnSpec<MissionRow, Record<string, never>> = {
  ...COLUMNS[0]!, priority: 0, text: true, cellKind: 'item', hideable: false, widest: ['Eastern Poland'],
  value: row => textValue(row.region.name, row.region.missing ?? 'Region name unavailable'),
};

const WINGS: DeclarativeColumnSpec<MissionRow, Record<string, never>> = {
  ...WINGS_COLUMN, priority: 1, numeric: true, format: 'integer', widest: ['999'], liveField: 'wings',
};

function missionColumn(mission: AirMissionChoice, at: number): DeclarativeColumnSpec<MissionRow, Record<string, never>> {
  const field = `m${mission.id}`;
  return {
    key: field,
    label: mission.name,
    as: 'faint',
    tone: 'grey',
    offered: OFFERED.number,
    priority: at + 2,
    numeric: true,
    format: 'integer',
    widest: ['9,999'],
    unit: 'aircraft',
    value: (row, live) => row.flown.has(mission.id) ? valueFromLive(live, liveKey(row.region.region, field)) : zeroValue(0),
  };
}

const columnsCache = new WeakMap<readonly AirMissionChoice[], readonly DeclarativeColumnSpec<MissionRow, Record<string, never>>[]>();

function missionColumns(reply: AirMissionsReply): readonly DeclarativeColumnSpec<MissionRow, Record<string, never>>[] {
  let held = columnsCache.get(reply.missions);
  if (!held) {
    held = [REGION_COLUMN, ...reply.missions.map(missionColumn), WINGS];
    columnsCache.set(reply.missions, held);
  }
  return held;
}

export const airMissionsProjection: TableProjection<Record<string, never>> = registerTableProjection(
  declareTableProjection<AirMissionsReply, MissionRow, Record<string, never>>({
    kind: 'air-missions',
    validate: validateAirMissions,
    columns: missionColumns,
    rows: reply => {
      if (reply.missing !== null) return { rows: [], empty: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
      const rows = reply.regions.map(region => ({ region, flown: new Set(region.missions) }));
      return {
        rows,
        ...(!rows.length ? { empty: 'No air missions at this date', emptyKind: 'empty' as const } : {}),
        warnings: reply.warnings,
      };
    },
    rowKey: row => row.region.region,
    rowName: row => row.region.name ?? `Region ${row.region.region}`,
    tooltipExtra: (_row, reply) => reply.readHour === null ? undefined : [{ label: 'Read', value: knownValue(formatDateOnly(reply.readHour)), divider: true }],
  }),
);

export const airMissionsDeclaration: TableWidgetDeclaration<Record<string, never>> = {
  family: 'table',
  kind: 'air-missions',
  label: 'Air missions by region',
  subject: 'air missions',
  emptyCampaign: 'Load a campaign to see air missions.',
  icon: 'chart-line',
  countries: { mode: 'single', choose: 'Choose a country to see its air missions.' },
  sources: ctx => {
    const params = ctx.countryTag ? { branchId: ctx.branch.id, country: countryFromKey(ctx.countryTag) } : null;
    return {
      owner: `air-missions:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${ctx.countryTag ?? ''}`,
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
              product: 'air-missions',
              followQuery: { panel: 'air-missions', ...params },
              params,
              validate: validateAirMissions,
            },
          }]
        : [],
    };
  },
  projection: airMissionsProjection,
  columns: [REGION_COLUMN],
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: TABLE_BASE },
  projectionOptions: () => NO_OPTIONS,
  jumpToDate: true,
};
