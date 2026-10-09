import type { WidgetCountryChipProps } from '../../dashboard/definitions';

export default function BombingCountryChip({ onSlot }: WidgetCountryChipProps) {
  return <span className="dash-widget-country-slot" ref={onSlot} />;
}
