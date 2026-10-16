import type { ParityList } from '../../dashboard/parityRows';
import { equipmentGroupValue, groupValue } from './producedData.ts';
import type { ProducedAt, ProducedModel } from './producedData';
import type { ProductionSelection } from './preparedProducedData';

const raw = (value: number) => Number.isNaN(value) ? null : String(value);
type Value = { key: string; amount: string | null; equipmentAmount: string | null };
// The history reply retains equipment-line subtotals only where the combined
// amount is unknown; that is also when the view shows them.
const values = (items: Value[]) => items.filter(item => item.amount !== '0')
  .map(item => ({ ...item, equipmentAmount: item.amount === null ? item.equipmentAmount : null }))
  .sort((left, right) => left.key < right.key ? -1 : left.key > right.key ? 1 : 0);

/** Canonical exact endpoint operands; names, runtime handles and page offsets are deferred. */
export function historyProductionParity(model: ProducedModel | null, ranked: readonly { tag: string; at: ProducedAt & { total: number } }[], complete: boolean): ParityList {
  return { kind: 'chart', label: 'Equipment produced', complete, total: ranked.length,
    rows: model ? ranked.map(({ tag, at }, index) => {
      const series = model.series.get(tag)!;
      return { key: tag, index, tag, total: String(at.total), values: values(model.reply.keys.map((key, index) => ({ key: key.key,
        amount: raw(groupValue(series, index, at.index)), equipmentAmount: raw(equipmentGroupValue(series, index, at.index)) }))) };
    }) : [] };
}

export function preparedProductionParity(page: ProductionSelection | undefined, complete: boolean): ParityList {
  return { kind: 'chart', label: 'Equipment produced', complete, total: page?.totalCountries ?? 0,
    rows: page?.countries.map((country, index) => ({ key: country.tag, index, tag: country.tag, total: country.total,
      values: values(country.values.map(value => ({ ...value }))) })) ?? [] };
}
