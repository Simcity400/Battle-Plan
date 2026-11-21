import CountryChip from '../../dashboard/CountryChip';
import { ChooseCountryChip } from '../../dashboard/DatedCountryChip';
import type { WidgetCountryChipProps } from '../../dashboard/definitions';

export default function StabilityCountryChip(props: WidgetCountryChipProps) {
  if (props.widget.options?.show !== 'stability' && props.widget.options?.show !== 'warSupport') return <CountryChip {...props} />;
  return Object.hasOwn(props.widget.options ?? {}, 'stabilityCountry') ? <span className="dash-widget-country-slot" ref={props.onSlot} /> : <ChooseCountryChip />;
}
