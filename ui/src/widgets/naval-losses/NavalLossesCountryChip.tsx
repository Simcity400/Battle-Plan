import CountryChip from '../../dashboard/CountryChip';
import type { WidgetCountryChipProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { OPTIONS } from './definition';

export default function NavalLossesCountryChip(props: WidgetCountryChipProps) {
  const options = resolveOptions(OPTIONS, props.widget.options);
  return options.show === 'parts' && options.kind === 'both'
    ? <span className="dash-widget-country-slot" ref={props.onSlot} /> : <CountryChip {...props} />;
}
