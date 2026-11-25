import { countryKey } from '../../dashboard/countryKey.ts';
import { resolveWindow } from '../../dashboard/model.ts';
import { artHoverText } from '../../families/art/adapters.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { artKindCell, newIdentityBoundary, resolveIdentity, resolveName, type IdentityBoundary } from '../../families/table/identityCell.ts';
import { LOG_TABLE_BASE, logCountryCell, logPairCell, logDateCell, logStatusCell, occurrenceSelection, pagedTailEntry, projectPagedSlots } from '../../families/table/logHelpers.ts';
import type { TableModel, TableProjection, TableRow } from '../../families/table/model.ts';
import { TABLE_SETTINGS_KEY, tableSettingsOf } from '../../families/table/settings.ts';
import { knownValue, notRecorded } from '../../families/value.ts';
import type { NavalBattleLogRow, NavalBattlesTailReply } from '../../generated/dashboard';
import { NAVAL_BATTLES_COLUMNS } from './definition.ts';

function battleRow(view: BoundedReadView, row: NavalBattleLogRow, boundary: IdentityBoundary, options: Options): TableRow {
  const place = row.province === null ? notRecorded('Province not recorded') : resolveName(view, 'province', String(row.province), { atHour: row.hour, boundary }).value;
  const name = place.kind === 'known' ? knownValue('Battle of ' + place.value) : place;
  const sides = row.countries.map((slots, i) => slots.map(slot => resolveIdentity(view, 'country', countryKey(slot), { atHour: row.countriesHour[i], boundary })));
  const flags = row.countries.map((slots, i) => ({ tag: slots.length ? countryKey(slots[0]!) : null, atHour: row.countriesHour[i] }));
  const title = name.kind === 'known' ? name.value : 'Battle name not recorded';
  const colors = sides.map(side => side[0]?.color);
  const live = row.result === 'ongoing';
  const status = live ? logStatusCell('Ongoing', 'running') : row.result === 'unknown' ? { value: notRecorded('Battle result not recorded') }
    : logStatusCell(row.result === 'attackerWon' ? 'Attacker won' : 'Defender won', 'finished');
  return {
    key: row.key, name: title, rowHour: row.hour, live, meta: row.hour,
    tooltipExtra: () => sides.map((side, i) => ({ label: i === 0 ? 'Attackers' : 'Defenders',
      value: side.length ? knownValue(side.map(who => who.name).join(', ')) : notRecorded('Country not recorded') })),
    tooltipScrollId: JSON.stringify([options.scope, row.detailId, row.detailHour ?? (options.following ? row.readHour : view.hour)]),
    cells: {
      date: logDateCell(row.hour),
      kind: artKindCell(view, { kind: 'sprite', key: 'GFX_naval_combat_icon', frame: 0 }, 'square', 'Naval battle', 'Battle artwork unavailable'),
      country: logCountryCell(view, flags, row.hour, boundary), item: { value: name },
      shipsSunk: logPairCell(row.shipsSunk, 'Ships sunk', colors, row.detailId.startsWith('combat:') ? 'Observed losses so far' : undefined),
      ships: logPairCell(row.ships, 'Ships'), status,
    },
  };
}

interface Options {
  readonly byDate: boolean;
  readonly scope: string;
  readonly following: boolean;
}

const sprites: Readonly<Record<string, string>> = {
  shipsSunk: 'GFX_naval_combat_icon',
  ships: 'GFX_naval_region_window_ship_icon',
};

const base = { columns: NAVAL_BATTLES_COLUMNS, dayDividersOnly: true, rowTimeBoundaries: false } as const;
export const navalBattlesProjection: TableProjection<Options> = registerFamilyProjection({
  kind: 'naval-battles', family: 'table',
  project(view, options): TableModel {
    const hour = view.hour;
    const source = view.rows<NavalBattleLogRow>('main');
    const refused = view.refused('main') ?? source?.refused;
    if (!source || refused) return { ...base, rows: [], hour, ...(refused ? { refused } : {}) };
    const boundary = newIdentityBoundary();
    const page = projectPagedSlots(source, hour, view.rowWindow, row => battleRow(view, row, boundary, options));
    const columns = NAVAL_BATTLES_COLUMNS.map(column => {
      const key = sprites[column.key];
      if (!key || column.headerGroup?.kind !== 'pair') return column;
      const art = view.art({ kind: 'sprite', key, frame: 0 });
      const icon = art.status === 'ready' ? { icon: art.url } : { iconMissing: artHoverText(art) ?? undefined };
      return { ...column, headerGroup: { ...column.headerGroup, ...icon } };
    });
    return { ...base, columns, ...page, dateGrouped: options.byDate, dayDividersOnly: options.byDate, hour, nextChangeAt: boundary.next,
      warnings: (source.rawHead as NavalBattlesTailReply | undefined)?.warnings,
      ...(page.slots.length ? {} : { empty: 'No recorded battles match these settings.' }) };
  },
});

export const navalBattlesDeclaration: TableWidgetDeclaration<Options> = {
  family: 'table', kind: 'naval-battles', label: 'Naval battles', subject: 'battles', icon: 'anchor',
  emptyCampaign: 'Load a campaign to see its naval battles.', countries: { mode: 'filter' }, columns: NAVAL_BATTLES_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: LOG_TABLE_BASE },
  sources(ctx) {
    const win = ctx.widget.time.mode === 'range' ? resolveWindow(ctx.widget, ctx.track, [ctx.track], ctx.branch.lastHour) : null;
    const from = win ? ctx.gateSpan(Math.max(Math.floor(win.firstHour), ctx.branch.firstHour), ctx.branch.lastHour)?.firstHour ?? null : null;
    const { sort } = tableSettingsOf(ctx.options[TABLE_SETTINGS_KEY], NAVAL_BATTLES_COLUMNS, LOG_TABLE_BASE);
    const ask = { ...occurrenceSelection(ctx), sort };
    return { owner: JSON.stringify([ctx.track.source.id, ctx.branch.id, 'naval-battles', from, ask]),
      entries: [pagedTailEntry('main', ctx.track.source, ctx.branch.id, ctx.track.axisOffset,
        { product: 'naval-battles-tail', branchId: ctx.branch.id, from, limit: 64, ask }, ctx.following)] };
  },
  projection: navalBattlesProjection, projectionOptions: ctx => ({ byDate: ctx.settings.sort.column === 'date', following: ctx.following,
    scope: JSON.stringify([ctx.widget.id, ctx.track.source.id, ctx.track.source.revision, ctx.branch.id]) }),
};
