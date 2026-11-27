import { countryKey } from '../../dashboard/countryKey.ts';
import { resolveWindow } from '../../dashboard/model.ts';
import { artHoverText } from '../../families/art/adapters.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { artKindCell, LOADING_NAME, newIdentityBoundary, resolveIdentity, resolveName, type IdentityBoundary } from '../../families/table/identityCell.ts';
import { LOG_TABLE_BASE, logCountryCell, logPairCell, logDateCell, logStatusCell, logWatchCell, occurrenceSelection, pagedTailEntry, projectPagedSlots } from '../../families/table/logHelpers.ts';
import type { TableModel, TableProjection, TableRow, TooltipExtraRow } from '../../families/table/model.ts';
import { TABLE_SETTINGS_KEY, tableSettingsOf } from '../../families/table/settings.ts';
import { knownValue, notRecorded, partialValue, pendingValue, type Value } from '../../families/value.ts';
import type { LandBattleLogRow, LandBattlesTailReply } from '../../generated/dashboard';
import { IC_UNRECORDED, LAND_BATTLES_COLUMNS, LOSSES_SO_FAR } from './definition.ts';

function sideParticipants(side: readonly ReturnType<typeof resolveIdentity>[], unknown: boolean): Value<string> {
  if (!side.length) return notRecorded('Country not recorded');
  const known: string[] = [];
  const reasons: string[] = [];
  let hasPending = false;
  for (const who of side) {
    if (who.value.kind === 'known') known.push(who.value.value);
    else if (who.value.kind === 'pending') {
      hasPending = true;
      if (!reasons.includes(who.value.reason)) reasons.push(who.value.reason);
    } else if (who.value.kind === 'notRecorded') {
      if (!reasons.includes(who.value.reason)) reasons.push(who.value.reason);
    }
  }
  if (unknown) reasons.push(reasons.length ? 'some participants on this side were not recorded' : 'Some participants on this side were not recorded');
  if (!known.length) {
    if (hasPending) return pendingValue(reasons.join('; '));
    return notRecorded(reasons.join('; ') || 'Country not recorded');
  }
  const text = known.join(', ');
  return reasons.length ? partialValue(text, reasons.join('; ')) : knownValue(text);
}

function battleSidesValue(sideValues: readonly Value<string>[]): Value<string> {
  const resolved: string[] = [];
  const reasons: string[] = [];
  let hasPending = false;
  for (const value of sideValues) {
    if (value.kind === 'known') {
      resolved.push(value.value);
    } else if (value.kind === 'partial') {
      resolved.push(value.value);
      if (!reasons.includes(value.reason)) reasons.push(value.reason);
    } else if (value.kind === 'pending') {
      hasPending = true;
      if (!reasons.includes(value.reason)) reasons.push(value.reason);
    } else if (value.kind === 'notRecorded') {
      if (!reasons.includes(value.reason)) reasons.push(value.reason);
    }
  }
  if (!resolved.length) {
    if (hasPending) return pendingValue(reasons.join('; '));
    return notRecorded(reasons.join('; ') || 'Country not recorded');
  }
  const text = resolved.join(' · ');
  return reasons.length ? partialValue(text, reasons.join('; ')) : knownValue(text);
}

function battleRow(view: BoundedReadView, row: LandBattleLogRow, boundary: IdentityBoundary): TableRow {
  const place = row.province === null ? notRecorded('Province not recorded') : resolveName(view, 'province', String(row.province), { atHour: row.hour, boundary }).value;
  const name = place.kind === 'known' ? knownValue('Battle of ' + place.value) : place;
  const sides = row.countries.map((slots, i) => slots.map(slot => resolveIdentity(view, 'country', countryKey(slot), { atHour: row.countriesHour[i], boundary })));
  const flagEntries = row.countries.flatMap<{ readonly tag: string | null; readonly atHour: number }>((slots, i) => (
    slots.length
      ? slots.map(slot => ({ tag: countryKey(slot), atHour: row.countriesHour[i]! }))
      : [{ tag: null, atHour: row.countriesHour[i]! }]
  ));
  const flagNames = sides.flatMap((side, i) => (
    side.length
      ? side.map(who => who.value.kind === 'known' ? who.value.value : (i === 0 ? 'Attacker' : 'Defender'))
      : [i === 0 ? 'Attacker' : 'Defender']
  ));
  const coalition = row.countries[0].length > 1 || row.countries[1].length > 1;
  const defenderStart = Math.max(1, row.countries[0].length);
  const countryCell = logCountryCell(view, flagEntries, row.hour, boundary);
  const countryFlags = countryCell.flags?.map((flag, idx) => ({
    ...flag,
    artName: flagNames[idx],
    ...(coalition && idx === defenderStart ? { separator: true } : {}),
  }));
  const sideValues = sides.map((side, i) => sideParticipants(side, row.countriesUnknown[i]!));
  const title = name.kind === 'known' ? name.value : name.kind === 'pending' ? LOADING_NAME : 'Battle name not recorded';
  const colors = sides.map(side => side[0]?.color);
  const live = row.result === 'ongoing';
  const status = live ? logStatusCell('Ongoing', 'running') : row.result === 'unknown' ? { value: notRecorded('Battle result not recorded') }
    : logStatusCell(row.result === 'attackerWon' ? 'Attacker won' : 'Defender won', 'finished');
  return {
    key: row.key, name: title, rowHour: row.hour, live, meta: { row, colors } satisfies BattleDetailSubject,
    tooltipExtra: (): TooltipExtraRow[] => sideValues.map((value, i) => ({
      label: i === 0 ? 'Attackers' : 'Defenders',
      value,
      ...(i === 0 ? { divider: true } : {}),
    })),
    cells: {
      date: logDateCell(row.hour),
      kind: artKindCell(view, { kind: 'sprite', key: 'GFX_landbattle_strip', frame: 0 }, 'square', 'Land battle', 'Battle artwork unavailable'),
      country: { ...countryCell, ...(countryFlags ? { flags: countryFlags } : {}), value: battleSidesValue(sideValues) },
      item: { value: name },
      place: { value: row.state === null ? notRecorded('Province state not recorded') : resolveName(view, 'state', String(row.state), { atHour: row.hour, boundary }).value },
      casualties: logPairCell(row.casualties, 'Casualties', colors, row.lossesSoFar ? LOSSES_SO_FAR : undefined),
      icLost: logPairCell(row.icLost, 'IC lost', colors, row.lossesSoFar ? LOSSES_SO_FAR : undefined, IC_UNRECORDED),
      divisions: logPairCell(row.divisions, 'Divisions'), status, action: logWatchCell(),
    },
  };
}

export interface BattleDetailSubject {
  readonly row: LandBattleLogRow;
  readonly colors: (string | undefined)[];
}

const sprites: Readonly<Record<string, string>> = {
  casualties: 'GFX_manpower_icon',
  icLost: 'GFX_industrial_capacity_icon',
  divisions: 'GFX_unit_infantry_icon_medium',
};

const base = { columns: LAND_BATTLES_COLUMNS, rowTimeBoundaries: false, rhythm: 'log' } as const;
export const landBattlesProjection: TableProjection<{ byDate: boolean }> = registerFamilyProjection({
  kind: 'land-battles', family: 'table',
  project(view, options): TableModel {
    const hour = view.hour;
    const source = view.rows<LandBattleLogRow>('main');
    const refused = view.refused('main') ?? source?.refused;
    if (!source || refused) return { ...base, rows: [], hour, ...(refused ? { refused } : {}) };
    const boundary = newIdentityBoundary();
    const page = projectPagedSlots(source, hour, view.rowWindow, row => battleRow(view, row, boundary));
    const columns = LAND_BATTLES_COLUMNS.map(column => {
      const key = sprites[column.key];
      if (!key || column.headerGroup?.kind !== 'pair') return column;
      const art = view.art({ kind: 'sprite', key, frame: 0 });
      const icon = art.status === 'ready' ? { icon: art.url } : { iconMissing: artHoverText(art) ?? undefined };
      return { ...column, headerGroup: { ...column.headerGroup, ...icon } };
    });
    return { ...base, columns, ...page, dateGrouped: options.byDate, hour, nextChangeAt: boundary.next,
      warnings: (source.rawHead as LandBattlesTailReply | undefined)?.warnings,
      ...(page.slots.length ? {} : { empty: 'No recorded battles match these settings.' }) };
  },
});

export const landBattlesDeclaration: TableWidgetDeclaration<{ byDate: boolean }> = {
  family: 'table', kind: 'land-battles', label: 'Land battles', subject: 'battles', icon: 'swords',
  emptyCampaign: 'Load a campaign to see its land battles.', countries: { mode: 'filter' }, columns: LAND_BATTLES_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: LOG_TABLE_BASE },
  sources(ctx) {
    const win = ctx.widget.time.mode === 'range' ? resolveWindow(ctx.widget, ctx.track, [ctx.track], ctx.branch.lastHour) : null;
    const from = win ? ctx.gateSpan(Math.max(Math.floor(win.firstHour), ctx.branch.firstHour), ctx.branch.lastHour)?.firstHour ?? null : null;
    const { sort } = tableSettingsOf(ctx.options[TABLE_SETTINGS_KEY], LAND_BATTLES_COLUMNS, LOG_TABLE_BASE);
    const ask = { ...occurrenceSelection(ctx), sort };
    return { owner: JSON.stringify([ctx.track.source.id, ctx.branch.id, 'land-battles', from, ask]),
      entries: [pagedTailEntry('main', ctx.track.source, ctx.branch.id, ctx.track.axisOffset,
        { product: 'land-battles-tail', branchId: ctx.branch.id, from, limit: 64, ask }, ctx.following)] };
  },
  projection: landBattlesProjection, projectionOptions: ctx => ({ byDate: ctx.settings.sort.column === 'date' }),
};
