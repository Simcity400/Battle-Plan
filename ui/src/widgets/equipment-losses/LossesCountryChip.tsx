import CountryChip from '../../dashboard/CountryChip';
import type { WidgetCountryChipProps } from '../../dashboard/definitions';
import { DETAIL } from './definition';

export default function LossesCountryChip(props: WidgetCountryChipProps) {
  return props.widget.options?.show === DETAIL.id
    ? <span className="dash-widget-country-slot" ref={props.onSlot} /> : <CountryChip {...props} />;
}
