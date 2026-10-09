import CountryChip from '../../dashboard/CountryChip';
import type { WidgetCountryChipProps } from '../../dashboard/definitions';

export default function MeasureCountryChip(props: WidgetCountryChipProps) {
  return props.widget.options?.show === 'income' || props.widget.options?.show === 'spending'
    ? <span className="dash-widget-country-slot" ref={props.onSlot} /> : <CountryChip {...props} />;
}
