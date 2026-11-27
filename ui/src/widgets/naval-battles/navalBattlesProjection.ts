import { countryKey } from '../../dashboard/countryKey.ts';
import { resolveWindow } from '../../dashboard/model.ts';
import { artHoverText } from '../../families/art/adapters.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { artKindCell, LOADING_NAME, newIdentityBoundary, resolveIdentity, resolveName, type IdentityBoundary } from '../../families/table/identityCell.ts';
import { LOG_TABLE_BASE, logCountryCell, logPairCell, logDateCell, logStatusCell, logWatchCell, occurrenceSelection, pagedTailEntry, projectPagedSlots } from '../../families/table/logHelpers.ts';
import type { TableCell, TableModel, TableProjection, TableRow, TooltipExtraRow } from '../../families/table/model.ts';
import { TABLE_SETTINGS_KEY, tableSettingsOf } from '../../families/table/settings.ts';
import { knownValue, notRecorded, partialValue, pendingValue, type Value } from '../../families/value.ts';
import type { NavalBattleLogRow, NavalBattlesTailReply } from '../../generated/dashboard';
import { NAVAL_BATTLES_COLUMNS } from './definition.ts';

const LEAST_SHARE = 0.03;

export interface NavalBattleSubject {
  readonly row: NavalBattleLogRow;
}

export interface NavalBattleClock {
  readonly liveEnd: number | null;
  readonly live: readonly (readonly [number, number])[];
  readonly result: { readonly handle: string; readonly from: number } | null;
  readonly span: { readonly first: number; readonly last: number } | null;
  readonly noteFrom: number | null;
}

export function navalBattleClock(row: NavalBattleLogRow, hour: number, firstHour: number, lastHour: number): NavalBattleClock {
  const ongoing = row.result === 'ongoing';
  const result = row.detailId.startsWith('result:') && row.resultHour !== null ? { handle: row.detailId.slice('result:'.length), from: row.resultHour } : null;
  const liveEnd = ongoing ? hour : row.lastLiveHour;
  const first = Math.max(liveEnd === null && result ? result.from : row.hour, firstHour);
  const ending = ongoing || liveEnd === null ? null : liveEnd + 1;
  const end = ongoing ? hour : result ? result.from : ending;
  const span = end === null ? null : { first, last: Math.max(first, Math.min(end, hour, lastHour)) };
  const noteFrom = !result && span !== null && ending !== null && ending <= span.last ? ending : null;
  return { liveEnd, live: liveEnd === null ? [] : [[first, Math.max(first, liveEnd)]], result, span, noteFrom };
}

function countValue(n: number | null, label: string, lowerBound?: string): Value<number> {
  if (n === null) return notRecorded(`${label} not recorded`);
  return lowerBound ? partialValue(n, lowerBound, { atLeast: true }) : knownValue(n);
}

function vesselValue(
  ships: number | null,
  convoys: number | null,
  shipLabel: string,
  convoyLabel: string,
  lowerBound?: string,
): Value<number | string> {
  if (ships === null && convoys === null) return notRecorded(`${shipLabel} not recorded`);
  if (ships === null) {
    if (convoys === 0) return notRecorded(`${shipLabel} not recorded`);
    const reason = lowerBound ? `${lowerBound}; ${shipLabel.toLowerCase()} not recorded` : `${shipLabel} not recorded`;
    return partialValue(`${convoys}c`, reason, { atLeast: true });
  }
  if (convoys === null) {
    if (ships === 0) return notRecorded(`${convoyLabel} not recorded`);
    const reason = lowerBound ? `${lowerBound}; ${convoyLabel.toLowerCase()} not recorded` : `${convoyLabel} not recorded`;
    return partialValue(ships, reason, { atLeast: true });
  }
  const raw: number | string = convoys === 0 ? ships : ships === 0 ? `${convoys}c` : `${ships}s+${convoys}c`;
  return lowerBound ? partialValue(raw, lowerBound, { atLeast: true }) : knownValue(raw);
}

function vesselPairCell(
  ships: readonly [number | null, number | null],
  convoys: readonly [number | null, number | null],
  shipLabel: string,
  convoyLabel: string,
  colors?: readonly (string | undefined)[],
  lowerBound?: string,
): TableCell {
  const totalShips = ships[0] === null || ships[1] === null ? null : ships[0] + ships[1];
  const totalConvoys = convoys[0] === null || convoys[1] === null ? null : convoys[0] + convoys[1];
  const totalValue = totalShips === null
    ? notRecorded(`${shipLabel} not recorded`)
    : totalConvoys === null
      ? notRecorded(`${convoyLabel} not recorded`)
      : vesselValue(totalShips, totalConvoys, shipLabel, convoyLabel, lowerBound);
  const cell: TableCell = {
    value: totalValue,
    pair: [
      vesselValue(ships[0], convoys[0], shipLabel, convoyLabel, lowerBound),
      vesselValue(ships[1], convoys[1], shipLabel, convoyLabel, lowerBound),
    ],
  };
  if (!colors) return cell;
  const sideA = ships[0] === null || convoys[0] === null ? null : ships[0] + convoys[0];
  const sideB = ships[1] === null || convoys[1] === null ? null : ships[1] + convoys[1];
  const total = sideA === null || sideB === null ? null : sideA + sideB;
  if (total === 0) return { ...cell, splitBar: [] };
  const share = sideA !== null && total !== null && total > 0 ? sideA / total : null;
  const fraction = share !== null && share > 0 && share < 1 ? Math.min(1 - LEAST_SHARE, Math.max(LEAST_SHARE, share)) : share;
  if (fraction === null || fraction < 0 || fraction > 1) {
    return { ...cell, splitBar: [{ fraction: 1, fill: '', hatched: true }] };
  }
  return {
    ...cell,
    splitBar: [fraction, 1 - fraction].map((part, i) => ({
      fraction: part,
      fill: colors[i] ?? '',
      hatched: !colors[i],
    })),
  };
}

function sideNames(side: readonly ReturnType<typeof resolveIdentity>[]): Value<string> {
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
  if (!known.length) {
    if (hasPending) return pendingValue(reasons.join('; '));
    return notRecorded(reasons.join('; ') || 'Country not recorded');
  }
  const text = known.join(', ');
  return reasons.length ? partialValue(text, reasons.join('; ')) : knownValue(text);
}

function battleRow(view: BoundedReadView, row: NavalBattleLogRow, boundary: IdentityBoundary, options: Options): TableRow {
  const place = row.province === null ? notRecorded('Province not recorded') : resolveName(view, 'province', String(row.province), { atHour: row.hour, boundary }).value;
  const name = place.kind === 'known' ? knownValue('Battle of ' + place.value) : place;
  const sides = row.countries.map((slots, i) => slots.map(slot => resolveIdentity(view, 'country', countryKey(slot), { atHour: row.countriesHour[i], boundary })));
  const flags = row.countries.map((slots, i) => ({ tag: slots.length ? countryKey(slots[0]!) : null, atHour: row.countriesHour[i] }));
  const title = name.kind === 'known' ? name.value : name.kind === 'pending' ? LOADING_NAME : 'Battle name not recorded';
  const colors = sides.map(side => side[0]?.color);
  const live = row.result === 'ongoing';
  const status = live ? logStatusCell('Ongoing', 'running') : row.result === 'unknown'
    ? { ...logStatusCell('Ended', 'finished'), tooltipRows: [{ label: 'Result', value: knownValue('Ended') }, { label: 'Winner', value: notRecorded('Winner not recorded') }] }
    : logStatusCell(row.result === 'attackerWon' ? 'Attacker won' : 'Defender won', 'finished');
  const lowerBound = row.detailId.startsWith('combat:') ? 'Observed losses so far' : undefined;
  const shipsSunk = vesselPairCell(row.shipsSunk, row.convoysSunk, 'Ships sunk', 'Convoys sunk', colors, lowerBound);
  const icLost = logPairCell(row.icLost, 'IC lost', colors, lowerBound);
  const ships = vesselPairCell(row.ships, row.convoys, 'Ships', 'Convoys');
  return {
    key: row.key, name: title, rowHour: row.hour, live, meta: { row } satisfies NavalBattleSubject,
    ...(row.result === 'unknown' ? { tooltipNote: 'The game keeps a winner only for battles a player takes part in' } : {}),
    tooltipExtra: () => sides.flatMap((side, i): TooltipExtraRow[] => {
      const convoyCount = row.convoys[i];
      const convoySunkCount = row.convoysSunk[i];
      const showConvoys = convoyCount !== 0 || convoySunkCount !== 0;
      const rows: TooltipExtraRow[] = [
        { label: i === 0 ? 'Attackers' : 'Defenders', divider: true,
          value: sideNames(side) },
        { label: 'Ships', value: countValue(row.ships[i], 'Ships'), indent: true },
      ];
      if (showConvoys) rows.push({ label: 'Convoys', value: countValue(convoyCount, 'Convoys'), indent: true });
      rows.push({ label: 'Ships sunk', value: countValue(row.shipsSunk[i], 'Ships sunk', lowerBound), indent: true });
      if (showConvoys) rows.push({ label: 'Convoys sunk', value: countValue(convoySunkCount, 'Convoys sunk', lowerBound), indent: true });
      rows.push({ label: 'IC lost', value: icLost.pair![i]!, indent: true });
      return rows;
    }),
    tooltipScrollId: JSON.stringify([options.scope, row.detailId, row.detailHour ?? (options.following ? row.readHour : view.hour)]),
    cells: {
      date: logDateCell(row.hour),
      kind: artKindCell(view, { kind: 'sprite', key: 'GFX_naval_combat_icon', frame: 0 }, 'square', 'Naval battle', 'Battle artwork unavailable'),
      country: logCountryCell(view, flags, row.hour, boundary), item: { value: name },
      shipsSunk: { ...shipsSunk, tooltipRows: [] }, icLost: { ...icLost, tooltipRows: [] }, ships: { ...ships, tooltipRows: [] }, status,
      action: logWatchCell(),
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
  icLost: 'GFX_industrial_capacity_icon',
  ships: 'GFX_ships_icon',
};

const base = { columns: NAVAL_BATTLES_COLUMNS, rowTimeBoundaries: false, rhythm: 'log' } as const;
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
    return { ...base, columns, ...page, dateGrouped: options.byDate, hour, nextChangeAt: boundary.next,
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
