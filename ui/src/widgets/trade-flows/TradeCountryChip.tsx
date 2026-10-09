import CountryChip from '../../dashboard/CountryChip';
import type { WidgetCountryChipProps } from '../../dashboard/definitions';

export default function TradeCountryChip(props: WidgetCountryChipProps) {
  return props.widget.options?.show === 'contributions' && props.widget.countries.kind === 'tags' && props.widget.countries.tags.length === 1
    ? <span className="dash-widget-country-slot" ref={props.onSlot} /> : <CountryChip {...props} />;
}
