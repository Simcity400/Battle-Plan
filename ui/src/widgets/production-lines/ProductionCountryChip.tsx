import CountryChip from '../../dashboard/CountryChip';
import type { WidgetCountryChipProps } from '../../dashboard/definitions';

export default function ProductionCountryChip(props: WidgetCountryChipProps) {
  return props.widget.options?.show === 'efficiency' || props.widget.options?.show === 'output'
    ? <span className="dash-widget-country-slot" ref={props.onSlot} /> : <CountryChip {...props} />;
}
