import { useMemo } from 'react';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { useProduct } from '../../dashboard/products.ts';
import type { GatedHour } from '../../dashboard/regionTime';
import { useSpriteArtwork } from '../../dashboard/useSpriteArtwork.ts';
import type { PickerContextAtHour, PickerList, TablePicker } from './picker.ts';

export default function ProductPicker({ spec, runtime, context, hour, selected, children }: {
  spec: TablePicker; runtime: WidgetRuntime; context: PickerContextAtHour; hour: GatedHour; selected?: string; children: PickerList;
}) {
  const { data, error } = useProduct<{ partial?: string | null }>(runtime.client, spec.request(context, hour), true, undefined, context.track.source);
  const items = useMemo(() => data === undefined ? [] : spec.items(data, context, hour, selected), [spec, data, context, hour, selected]);
  const sprite = useSpriteArtwork(runtime, context.track, items.map(item => item.sprite ? { key: item.sprite, frame: 0 } : null), true);
  if (error || data === undefined) return <div role="status">{error?.message ?? 'Loading choices...'}</div>;
  return children(items.map(item => item.sprite ? { ...item, icon: sprite({ key: item.sprite, frame: 0 }) ?? undefined } : item), data.partial);
}
