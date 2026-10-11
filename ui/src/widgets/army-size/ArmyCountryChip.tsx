import CountryChip from '../../dashboard/CountryChip';
import type { WidgetCountryChipProps } from '../../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { normaliseArmyOptions, OPTIONS } from './definition';
import { opaqueTag, signed32 } from '../../dashboard/validation';

export function armyCountrySelection(widget: WidgetSpec, track: BoundTrack | undefined) {
  const tag = widget.countries.kind === 'tags' && widget.countries.tags.length === 1 ? widget.countries.tags[0] : null;
  const hour = widget.options?.drillOriginHour;
  return track && opaqueTag(tag) && signed32(hour) && hour >= track.branch.firstHour && hour <= track.branch.lastHour ? { tag, hour } : null;
}

export default function ArmyCountryChip(props: WidgetCountryChipProps) {
  return resolveOptions(OPTIONS, normaliseArmyOptions(props.widget.options)).view === 'breakdown' ? <span className="dash-widget-country-slot" ref={props.onSlot} /> : <CountryChip {...props} />;
}
