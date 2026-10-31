import { runAt } from '../../charts/series.ts';
import { readAt } from '../../dashboard/reads.ts';
import type { EquipmentStockpileReply } from '../../generated/dashboard';
import type { ParityList } from '../../dashboard/parityRows';
import { balanceMode, balanceOperandsAt, dailyAt, dailyIcAt, stockAt, variantName } from './stockpileData.ts';
import type { StockGroup } from './stockpileData';
import type { PreparedStockpileReply } from './stockpileCatalogue';

/** Exact displayed facts require one dated reply, independent of chart density. */
export function stockpileParityModel(reply: EquipmentStockpileReply | undefined, groups: readonly StockGroup[], hour: number, label: string): ParityList {
  const complete = !!reply && (!(reply as PreparedStockpileReply).prepared || (reply as PreparedStockpileReply).prepared!.hour === hour);
  const raw = (value: bigint | null) => value === null ? null : String(value);
  const rows = reply ? groups.map(group => {
    const logistics = balanceOperandsAt(reply, group, hour);
    return { key: group.key, label: group.label, kind: group.kind, stockRaw: raw(stockAt(reply, group, hour)),
      dailyRaw: raw(dailyAt(reply, group, hour)), dailyIcRaw: raw(dailyIcAt(reply, group, hour)),
      balanceRaw: raw(logistics?.balance ?? null), needRaw: raw(logistics?.need ?? null), availableRaw: raw(logistics?.available ?? null), needNote: logistics?.needNote ?? null,
      variants: group.variants.map(variant => ({ id: variant.id, token: variant.token, name: variantName(variant, hour),
        stockRaw: raw(stockAt(reply, { variants: [variant] }, hour)), dailyRaw: raw(dailyAt(reply, { variants: [variant] }, hour)),
        dailyIcRaw: raw(dailyIcAt(reply, { variants: [variant] }, hour)),
        costLowRaw: runAt(variant.unitCostLow, hour)?.amountRaw ?? null, costHighRaw: runAt(variant.unitCostHigh, hour)?.amountRaw ?? null })) };
  }) : [];
  return { kind: 'chart', label, scope: 'displayed-hour', complete, total: rows.length, rows, hour,
    balanceMode: reply ? balanceMode(reply, hour) : null, stockRead: reply ? readAt(reply.reads, hour) : null,
    lineRead: reply ? readAt(reply.lineReads, hour) : null, balanceRead: reply ? readAt(reply.logisticsReads, hour) : null };
}
