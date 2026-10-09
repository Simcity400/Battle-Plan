import CountryChip from '../../dashboard/CountryChip';
import type { WidgetCountryChipProps } from '../../dashboard/definitions';
import { BREAKDOWN } from './definition';

export default function StockpileCountryChip(props: WidgetCountryChipProps) {
  return props.widget.options?.show === BREAKDOWN.id ? <span className="dash-widget-country-slot" ref={props.onSlot} /> : <CountryChip {...props} />;
}
