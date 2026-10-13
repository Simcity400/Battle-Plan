import CountryChip, { breakdownCountryChip } from '../../dashboard/CountryChip';
import type { WidgetCountryChipProps } from '../../dashboard/definitions';

export default function StabilityCountryChip(props: WidgetCountryChipProps) {
  return props.widget.options?.show === 'stability' || props.widget.options?.show === 'warSupport'
    ? breakdownCountryChip(props, 'stabilityCountry') : <CountryChip {...props} />;
}
