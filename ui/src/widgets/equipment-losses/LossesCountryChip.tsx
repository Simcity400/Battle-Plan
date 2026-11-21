import CountryChip from '../../dashboard/CountryChip';
import { ChooseCountryChip } from '../../dashboard/DatedCountryChip';
import type { WidgetCountryChipProps } from '../../dashboard/definitions';
import { DETAIL } from './definition';

export default function LossesCountryChip(props: WidgetCountryChipProps) {
  if (props.widget.options?.show !== DETAIL.id) return <CountryChip {...props} />;
  return Object.hasOwn(props.widget.options ?? {}, 'lossCountry') ? <span className="dash-widget-country-slot" ref={props.onSlot} /> : <ChooseCountryChip />;
}
