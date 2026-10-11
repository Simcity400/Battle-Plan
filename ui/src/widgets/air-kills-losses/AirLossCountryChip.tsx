import CountryChip from '../../dashboard/CountryChip';
import type { WidgetCountryChipProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { normaliseAirOptions, OPTIONS } from './definition';

export default function AirLossCountryChip(props: WidgetCountryChipProps) {
  return resolveOptions(OPTIONS, normaliseAirOptions(props.widget.options)).view === 'breakdown' ? <span className="dash-widget-country-slot" ref={props.onSlot} /> : <CountryChip {...props} />;
}
