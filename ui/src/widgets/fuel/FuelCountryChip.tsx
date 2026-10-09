import CountryChip from '../../dashboard/CountryChip';
import type { WidgetCountryChipProps } from '../../dashboard/definitions';
import { BREAKDOWNS } from './definition';

export default function FuelCountryChip(props: WidgetCountryChipProps) {
  const selected = props.widget.options?.fuelCountry;
  return typeof selected === 'string' && selected.length > 0 && BREAKDOWNS.some(item => item.id === props.widget.options?.show)
    ? <span className="dash-widget-country-slot" ref={props.onSlot} /> : <CountryChip {...props} />;
}
