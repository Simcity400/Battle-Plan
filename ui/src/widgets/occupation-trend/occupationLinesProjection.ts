import { countryKey } from '../../dashboard/countryKey.ts';
import { repliedCells } from '../../dashboard/follow/liveCells.ts';
import { FutureDataError, type BoundedReadView } from '../../families/contract.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { createCountryLinesView, type CountryLine, type CountryLinesOptions } from '../../families/time-chart/countryLines.ts';
import type { ChartHoverDetail } from '../../families/time-chart/model.ts';
import { countryNameAt } from '../../families/time-chart/seriesHelpers.ts';
import type { LiveValues } from '../../charts/live';
import type { OccupationDetailReply } from '../../generated/dashboard';
import { formatAmount } from '../../charts/valueAxis.ts';
import {
  SERIES,
  heldCoverage,
  measureName,
  occupationPercent,
  otherMeasure,
  rankingMeasure,
  totalsMissing,
  totalsOf,
  validateOccupationDetail,
  type OccupationMeasure,
  type ShownMeasure,
} from './occupationTrendData.ts';

const shownOf = (options: Readonly<Record<string, unknown>>) => (options.measure === 'compliance' || options.measure === 'both' ? options.measure : 'resistance') as ShownMeasure;
const rankOf = (options: Readonly<Record<string, unknown>>) => rankingMeasure(shownOf(options), options.rankingMeasure === 'compliance' ? 'compliance' : 'resistance');

export interface OccupationHoverSettings {
  readonly shown: ShownMeasure;
  readonly rank: OccupationMeasure;
  readonly collaboration: boolean;
  readonly digits: number;
}

export function occupationHoverItems(reply: OccupationDetailReply, values: LiveValues<number>, lines: readonly CountryLine[],
  settings: OccupationHoverSettings, nameOf: (tag: string) => string): TooltipListItem[] {
  const { shown, rank, collaboration } = settings;
  const percent = (raw: string) => occupationPercent(raw, settings.digits);
  const items: TooltipListItem[] = [];
  for (const line of lines) {
    const totals = totalsOf(values, line.tag);
    const row = reply.occupiers.find(item => countryKey(item.country) === line.tag);
    if (!totals) {
      items.push({ label: line.name, swatch: line.color, value: 'Not recorded', tone: 'muted', missingReason: row?.missing ?? totalsMissing(values, line.tag), divider: items.length > 0 });
      continue;
    }
    const value = (measure: OccupationMeasure) => percent(measure === 'resistance' ? totals.resistanceRaw : totals.complianceRaw);
    items.push({ label: line.name, swatch: line.color, value: value(rank), strong: true, divider: items.length > 0 });
    if (shown === 'both') items.push({ label: `Average ${measureName(otherMeasure(rank)).toLowerCase()}`, value: value(otherMeasure(rank)), indent: true });
    items.push({ label: 'States', value: heldCoverage(totals), indent: true });
    if (!collaboration) continue;
    if (reply.collaborationMissing !== null) {
      items.push({ label: 'Country collaboration', value: reply.collaborationMissing, tone: 'muted', indent: true });
      continue;
    }
    for (const item of row?.collaboration ?? []) {
      items.push({
        label: `Country collaboration: ${item.tag === null ? 'Country name unavailable' : nameOf(item.tag)}`,
        value: item.progressRaw === null ? 'Not recorded' : `${formatAmount((BigInt(item.progressRaw) * 100n).toString(), 5, 5)}%`,
        indent: true,
      });
    }
  }
  return items.length ? items : [{ label: 'Not recorded', tone: 'muted' }];
}

function occupationHoverDetail(readView: BoundedReadView, lines: readonly CountryLine[], options: CountryLinesOptions): ChartHoverDetail | null {
  let hover;
  try {
    hover = readView.hover<OccupationDetailReply>('detail');
  } catch (error) {
    if (error instanceof FutureDataError) return null;
    throw error;
  }
  if (!hover) return null;
  const at = hover.hour;
  const settings = { shown: shownOf(options.options), rank: rankOf(options.options), collaboration: options.options.collaboration === true, digits: options.digits ?? 5 };
  const items = occupationHoverItems(hover.data, repliedCells(hover.data).read(), lines, settings, tag => countryNameAt(readView, tag, at));
  return { hour: at, items, replacesSeries: true };
}

export const { projection: occupationLinesProjection, declaration: occupationLinesDeclaration } = createCountryLinesView({
  kind: 'occupation-trend',
  view: 'lines',
  title: 'Occupation trend',
  subject: 'occupation trend',
  format: 'percent',
  step: true,
  measure: options => ({ kind: SERIES, measure: rankOf(options), decimals: 5 }),
  second: options => {
    if (shownOf(options) !== 'both') return null;
    const rank = rankOf(options);
    return { measure: otherMeasure(rank), legend: [measureName(rank), measureName(otherMeasure(rank))] };
  },
  digits: options => Number(options.digits ?? 2),
  hover: {
    entry: ctx => ({
      name: 'detail',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'exact-hover',
        product: 'occupation-at',
        params: { branchId: ctx.branch.id, countries: [], collaboration: ctx.options.collaboration === true },
        validate: validateOccupationDetail,
      },
    }),
    detail: occupationHoverDetail,
  },
});
