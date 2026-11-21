import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { contentKey, setToken } from '../../dashboard/options.ts';
import { pinnedBreakdown } from '../../charts/drill/drillState.ts';
import { datedCountryToken } from '../../dashboard/datedCountry.ts';
import type { DatedCountrySelection } from '../../dashboard/datedCountry';
import { BREAKDOWN } from './definition.ts';
import { choiceKey, choiceRow, labelAt, stockChoices } from './stockpileData.ts';
import type { Grouping, StockGroup } from './stockpileData';

export type StockpileMode = 'balance' | 'stock';

export function pinnedGrouping(options: WidgetSpec['options']): Grouping {
  const saved = options?.show === BREAKDOWN.id ? options.grouping : undefined;
  return saved === 'variant' || saved === 'domain' ? saved : 'type';
}

export function stockpileDrillGroup(groups: readonly StockGroup[], stored: string): StockGroup | undefined {
  const key = contentKey(stored);
  return stockChoices(groups, 'equipment').lines(key) ? groups.find(group => group.key === choiceRow(key)) : undefined;
}

export function pinStockpileDrill(widget: WidgetSpec, track: Pick<BoundTrack, 'id' | 'branch'>, tag: string,
  group: StockGroup, grouping: Grouping, mode: StockpileMode, hour: number, follow: boolean, country: DatedCountrySelection): WidgetSpec | null {
  const key = choiceKey(group, hour);
  if (key == null) return null;
  const pinned = pinnedBreakdown({ ...widget, countries: { kind: 'tags', tags: [tag] } },
    { ...BREAKDOWN, label: labelAt(group, grouping, hour) }, track.id);
  pinned.options = { ...pinned.options, grouping, partGroup: setToken(key), partMode: mode, stockpileCountry: datedCountryToken(country) };
  pinned.time = follow ? { mode: 'follow' } : { mode: 'pin', windows: { [track.id]: {
    firstOffset: Math.floor(hour) - track.branch.firstHour, lastOffset: Math.floor(hour) - track.branch.firstHour } } };
  return pinned;
}
