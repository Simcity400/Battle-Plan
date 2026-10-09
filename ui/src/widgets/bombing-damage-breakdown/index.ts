import type { WidgetRenderer } from '../../dashboard/definitions';
import BombingDamageBreakdownWidget from './BombingDamageBreakdownWidget';
import BombingCountryChip from './BombingCountryChip';
export default { render: BombingDamageBreakdownWidget, countryChip: BombingCountryChip } satisfies WidgetRenderer;
