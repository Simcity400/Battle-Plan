import type { ShipsSunkReply, ShipsSunkRow } from '../../generated/dashboard';
import type { LogRow } from '../../charts/log/logData.ts';
import { newestFirst } from '../../charts/log/logData.ts';
import type { CountryRole } from './shipsSunkData.ts';
import { shipLogRows } from './shipsSunkData.ts';
import type { ShipsSunkWindowReply } from './useShipsSunk';

export interface SinkingItem extends ShipsSunkRow { index: number; windowIndex: number; page: ShipsSunkReply }
/** Each row keeps the dictionary that supplied it and its immutable scan index. */
export function sinkingWindowRows(pages: ReadonlyMap<number, ShipsSunkWindowReply>, hour: number,
  classes: readonly string[], role: CountryRole, country: (tag: string | null, hour: number) => boolean,
  first: number | null, campaign: string): LogRow<SinkingItem>[] {
  return newestFirst([...pages.values()].flatMap(page => shipLogRows(page.reply, hour, classes, role, country, first, undefined, campaign)
    .map(row => { const index = page.indices[Number(row.key)]; return { key: String(index), hour: row.hour,
      item: { ...row.item, index, windowIndex: page.window, page: page.reply } }; })));
}
