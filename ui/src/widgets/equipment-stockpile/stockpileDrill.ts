import type { GatedTime } from '../../dashboard/regionTime';
import type { PinnedSpec } from '../../charts/drill/drillState';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { contentKey, setToken } from '../../dashboard/options.ts';
import { pinnedBreakdown } from '../../charts/drill/drillState.ts';
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
  group: StockGroup, grouping: Grouping, mode: StockpileMode, hour: number, time: GatedTime, country: string): PinnedSpec | null {
  const key = choiceKey(group, hour);
  if (key == null) return null;
  const pinned = pinnedBreakdown({ ...widget, countries: { kind: 'tags', tags: [tag] } },
    { ...BREAKDOWN, label: labelAt(group, grouping, hour) }, track.id, time);
  pinned.options = { ...pinned.options, grouping, partGroup: setToken(key), partMode: mode, stockpileCountry: country };
  return pinned;
}
