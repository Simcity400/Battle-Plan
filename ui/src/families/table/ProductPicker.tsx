import { useMemo } from 'react';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt.ts';
import { useProduct } from '../../dashboard/products.ts';
import { failureText } from '../../dashboard/queryError.ts';
import type { GatedHour } from '../../dashboard/regionTime';
import { useSpriteArtwork } from '../../dashboard/useSpriteArtwork.ts';
import type { PickerContextAtHour, PickerList, TablePicker } from './picker.ts';

const NO_ITEMS: readonly never[] = [];

export default function ProductPicker({ spec, runtime, context, hour, selected, children }: {
  spec: TablePicker; runtime: WidgetRuntime; context: PickerContextAtHour; hour: GatedHour; selected?: string; children: PickerList;
}) {
  const { data, error } = useProduct<{ partial?: string | null }>(runtime.client, spec.request(context, hour), true, undefined, context.track.source);
  const items = useMemo(() => data === undefined ? NO_ITEMS : spec.items(data, context, hour, selected), [spec, data, context, hour, selected]);
  const sprite = useSpriteArtwork(runtime, context.track, items.map(item => item.sprite ? { key: item.sprite, frame: 0 } : null), true);
  const wanted = useMemo(() => [...new Set(items.flatMap(item => item.country ? [item.country] : []))].map(tag => ({ hour, tag })), [items, hour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, context.track, wanted, true);
  const shown = useMemo(() => {
    const label = labelAt(hour);
    return items.map(item => {
      const country = item.country ? label(item.country) : null;
      const icon = item.sprite ? sprite({ key: item.sprite, frame: 0 }) ?? undefined : item.icon;
      return { ...item, ...(icon ? { icon } : {}), ...(country?.known && country.name ? { countryName: country.name } : {}), ...(country?.flag ? { countryFlag: country.flag } : {}) };
    });
  }, [items, labelAt, hour, sprite]);
  if (error) return children(NO_ITEMS, { kind: 'failed', message: failureText(error, 'The choices could not be read.') });
  if (data === undefined) return children(NO_ITEMS, { kind: 'loading' });
  return children(shown, { kind: 'ready', notice: data.partial });
}
