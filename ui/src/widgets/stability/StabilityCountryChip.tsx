import CountryChip from '../../dashboard/CountryChip';
import type { WidgetCountryChipProps } from '../../dashboard/definitions';

export default function StabilityCountryChip(props: WidgetCountryChipProps) {
  return props.widget.options?.show === 'stability' || props.widget.options?.show === 'warSupport'
    ? <span className="dash-widget-country-slot" ref={props.onSlot} /> : <CountryChip {...props} />;
}
