import { countryKey } from '../../dashboard/countryKey.ts';
import { FutureDataError, type BoundedReadView } from '../../families/contract.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { createCountryLinesView, type CountryLine } from '../../families/time-chart/countryLines.ts';
import type { ChartHoverDetail } from '../../families/time-chart/model.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { NuclearArsenalReply, NukeCountry } from '../../generated/dashboard';
import { typeName, validateNuclearArsenal } from './nuclearArsenalData.ts';

function rowForLine(reply: NuclearArsenalReply, key: string): NukeCountry | undefined {
  return reply.countries.find(row => (row.slot === null ? row.tag : countryKey(row.slot)) === key || row.tag === key);
}

export function nuclearHoverItems(reply: NuclearArsenalReply, lines: readonly CountryLine[]): TooltipListItem[] {
  if (reply.missing !== null) {
    return [{ label: 'Nuclear stockpile', missingReason: reply.missing, tone: 'muted' }];
  }
  const typeByOrdinal = new Map(reply.types.map(type => [type.ordinal, typeName(type)]));
  const items: TooltipListItem[] = [];
  for (const line of lines) {
    const row = rowForLine(reply, line.tag);
    if (!row) {
      items.push({ label: line.name, swatch: line.color, value: formatScalar(0, 'integer'), divider: items.length > 0 });
      continue;
    }
    items.push({
      label: line.name,
      swatch: line.color,
      value: formatScalar(row.total, 'integer'),
      strong: true,
      divider: items.length > 0,
    });
    for (const count of row.counts) {
      if (count.value <= 0) continue;
      const label = typeByOrdinal.get(count.ordinal) ?? `Unnamed nuclear type ${count.ordinal}`;
      const value = count.ready > 0
        ? `${formatScalar(count.value, 'integer')} (${formatScalar(count.ready, 'integer')} ready)`
        : formatScalar(count.value, 'integer');
      items.push({ label, value, indent: true });
    }
  }
  return items;
}

function nuclearHoverDetail(readView: BoundedReadView, lines: readonly CountryLine[]): ChartHoverDetail | null {
  let hover;
  try {
    hover = readView.hover<NuclearArsenalReply>('detail');
  } catch (error) {
    if (error instanceof FutureDataError) return null;
    throw error;
  }
  if (!hover) return null;
  const items = nuclearHoverItems(hover.data, lines);
  return items.length ? { hour: hover.hour, items, replacesSeries: true } : null;
}

export const { projection: nuclearArsenalLinesProjection, declaration: nuclearArsenalLinesDeclaration } = createCountryLinesView({
  kind: 'nuclear-arsenal',
  view: 'lines',
  title: 'Nuclear arsenal',
  subject: 'nuclear stockpiles',
  format: 'number',
  step: true,
  digits: () => 0,
  measure: () => ({ kind: 'country-nuke-totals', measure: 'total', decimals: 0 }),
  hover: {
    entry: ctx => ({
      name: 'detail',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'exact-hover',
        product: 'nuclear-arsenal',
        params: { branchId: ctx.branch.id, countries: [] },
        validate: validateNuclearArsenal,
      },
    }),
    detail: nuclearHoverDetail,
  },
});
