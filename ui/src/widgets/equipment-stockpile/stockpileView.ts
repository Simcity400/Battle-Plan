/** The stockpile table's rows: each equipment type with anything at the hour, led by
 * the game's surplus or shortage (what its Logistics screen shows: available minus
 * needed), with the models held or being made and the output per day. Stock appears
 * where it says something the surplus does not. */
import { runAt } from '../../charts/series.ts';
import { formatAmount } from '../../charts/valueAxis.ts';
import type { EquipmentStockpileReply } from '../../generated/dashboard';
import { balanceOperandsAt, dailyAt, dailyIcAt, shareOf, stockAt, typesComplete, variantName } from './stockpileData.ts';
import type { BalanceMode, StockGroup } from './stockpileData';

export type StockCount = 'units' | 'ic';

export interface StockRow {
  group: StockGroup;
  /** The models held or being made, and where it differs, stock and availability. */
  models: string;
  /** The game's surplus (positive) or shortage (negative); null where not known. */
  surplus: bigint | null;
  /** The surplus as a share of need, where the need is positive. */
  share: number | null;
  stock: bigint | null;
  /** Units (or IC) a day its lines make. */
  output: bigint | null;
}

/** A value that is not known while its table was read: unknown, never empty. */
const unknownWhere = (spans: EquipmentStockpileReply['stockHours'], hour: number, value: bigint | null) => value == null && runAt(spans, hour) != null;

/** Every type with anything at the hour (stock, output, need or a surplus, known or
 * not), how many types are read to have nothing then, and how many show nothing only
 * because a value was not read (`unrecorded`): those are never said to be empty. */
export function stockRows(reply: EquipmentStockpileReply, groups: readonly StockGroup[], hour: number, mode: BalanceMode, count: StockCount)
  : { rows: StockRow[]; quiet: number; unrecorded: number } {
  const amount = (raw: bigint) => formatAmount(raw, reply.decimals, 0);
  const rows: StockRow[] = [];
  let quiet = 0, unrecorded = 0;
  for (const group of groups) {
    const stock = stockAt(reply, group, hour);
    const output = count === 'ic' ? dailyIcAt(reply, group, hour) : dailyAt(reply, group, hour);
    const player = mode === 'player';
    const logistics = player ? balanceOperandsAt(reply, group, hour) : null;
    const complete = !player || typesComplete(reply, group, hour);
    const surplus = complete ? logistics?.balance ?? null : null;
    const holds = (stock != null && stock !== 0n) || unknownWhere(reply.stockHours, hour, stock)
      || (output != null && output !== 0n) || unknownWhere(reply.lineHours, hour, output)
      || (player && (!complete || (surplus != null && surplus !== 0n) || (logistics?.need ?? 0n) !== 0n || unknownWhere(reply.balanceHours, hour, surplus)));
    if (!holds) {
      if (stock != null && output != null && (!player || surplus != null && logistics?.need != null)) quiet++; else unrecorded++;
      continue;
    }
    const models = group.variants.map(variant => ({ name: variantName(variant, hour), stock: stockAt(reply, { variants: [variant] }, hour),
      output: dailyAt(reply, { variants: [variant] }, hour) }))
      .filter(model => model.stock !== 0n || (model.output != null && model.output !== 0n))
      .sort((a, b) => (a.stock == null ? 1 : 0) - (b.stock == null ? 1 : 0) || (a.stock === b.stock ? a.name.localeCompare(b.name) : (a.stock ?? 0n) > (b.stock ?? 0n) ? -1 : 1));
    // Where the game's availability is not the stockpile (convoys, supply trucks and
    // trains, as content marks them), both are named.
    const special = reply.types.find(type => type.key === group.key)?.needNote != null;
    const available = logistics?.available ?? null;
    const differs = player && special && available != null && stock != null && available !== stock;
    // Counts lead their model, because model names carry numbers of their own. A single
    // model is named alone where its stock would only repeat the row's figure.
    const named = models.length === 1 ? models[0].name
      : models.map(model => model.stock == null ? `${model.name} (not recorded)` : `${amount(model.stock)} ${model.name}`).join(' · ');
    const held = differs ? `${amount(stock)} in stock, ${amount(available)} available`
      : player && models.length === 1 && models[0].stock !== surplus
        ? models[0].stock == null ? 'stock not recorded' : `${amount(models[0].stock)} in stock` : '';
    const text = [named, held].filter(Boolean).join(' · ');
    rows.push({ group, models: text, surplus, share: complete ? shareOf(logistics) : null, stock, output });
  }
  return { rows, quiet, unrecorded };
}

export const stockRowRevision = (row: StockRow) => [row.group.label, row.models, row.surplus, row.share, row.stock, row.output];
