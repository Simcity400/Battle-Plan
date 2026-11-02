import type { ShipsSunkReply, ShipsSunkRow } from '../../generated/dashboard';
import type { LogRow } from '../../charts/log/logData.ts';
import { newestFirst } from '../../charts/log/logData.ts';
import type { CountryRole } from './shipsSunkData.ts';
import { shipLogRows } from './shipsSunkData.ts';
import type { ShipsSunkWindowReply } from './useShipsSunk';
import { LogPlayback } from '../../charts/log/logPlayback.ts';

export interface SinkingItem extends ShipsSunkRow { index: number; windowIndex: number; page: ShipsSunkReply }
type SinkingPages = ReadonlyMap<number, ShipsSunkWindowReply>;
type CountryFilter = (tag: string | null, hour: number) => boolean;
const playbacks = new WeakMap<SinkingPages, WeakMap<CountryFilter, Map<string, LogPlayback<SinkingItem>>>>();
/** Each row keeps the dictionary that supplied it and its immutable scan index. */
export function sinkingWindowRows(pages: ReadonlyMap<number, ShipsSunkWindowReply>, hour: number,
  classes: readonly string[], role: CountryRole, country: (tag: string | null, hour: number) => boolean,
  first: number | null, campaign: string): ReadonlyArray<LogRow<SinkingItem>> {
  let byCountry = playbacks.get(pages);
  if (!byCountry) playbacks.set(pages, byCountry = new WeakMap());
  let bySettings = byCountry.get(country);
  if (!bySettings) byCountry.set(country, bySettings = new Map());
  const key = JSON.stringify([classes, role, first, campaign]);
  let playback = bySettings.get(key);
  if (!playback) {
    const rows = newestFirst([...pages.values()].flatMap(page => shipLogRows(page.reply, Infinity, classes, role, country, first, undefined, campaign)
      .map(row => { const index = page.indices[Number(row.key)]; return { key: String(index), hour: row.hour,
        item: { ...row.item, index, windowIndex: page.window, page: page.reply } }; })));
    playback = new LogPlayback(rows, row => Math.max(row.hour, row.item.knownAt));
    bySettings.set(key, playback);
  }
  return playback.at(hour);
}
