import { Icon } from '../components/Icons';
import type { WidgetMeta, WidgetPickerItemsProps } from './definitions';
import type { WidgetSpec } from './model';

/** Shared button for a widget's default entry and content-derived presets. */
export default function WidgetPickerItem({ definition, disabled, reason = null, initial, onAdd }: {
  definition: WidgetMeta; disabled: boolean; reason?: string | null;
  initial?: Pick<WidgetSpec, 'title' | 'options'>; onAdd: WidgetPickerItemsProps['onAdd'];
}) {
  return <button type="button" disabled={disabled || reason !== null} data-tooltip={reason ?? undefined} onClick={() => onAdd(definition.kind, initial)}>
    <span className="dash-widget-icon" aria-hidden="true"><Icon name={definition.icon} /></span>
    <span><strong>{definition.title}</strong><small>{reason ?? [definition.category, definition.description].filter(Boolean).join(' · ')}</small></span><Icon name="plus" />
  </button>;
}
