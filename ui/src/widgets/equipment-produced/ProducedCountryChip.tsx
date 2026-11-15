import CountryChip from '../../dashboard/CountryChip';
import type { WidgetCountryChipProps } from '../../dashboard/definitions';

/** A pinned breakdown publishes the holder and date its body shows here; otherwise the countries chosen. */
export default function ProducedCountryChip(props: WidgetCountryChipProps) {
  return props.widget.options?.show === 'breakdown' ? <span className="dash-widget-country-slot" ref={props.onSlot} /> : <CountryChip {...props} />;
}
