import CountryChip from '../../dashboard/CountryChip';
import type { WidgetCountryChipProps } from '../../dashboard/definitions';
import { BREAKDOWN } from './definition';

export default function ResourceCountryChip(props: WidgetCountryChipProps) {
  return props.widget.options?.show === BREAKDOWN.id && (props.widget.options.resourceCountry
    || props.widget.countries.kind === 'tags' && props.widget.countries.tags.length === 1)
    ? <span className="dash-widget-country-slot" ref={props.onSlot} /> : <CountryChip {...props} />;
}
