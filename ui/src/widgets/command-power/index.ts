import CountryChip from '../../dashboard/CountryChip';
import type { WidgetRenderer } from '../../dashboard/definitions';
import CommandPowerWidget from './CommandPowerWidget.tsx';

export default { countryChip: CountryChip, render: CommandPowerWidget } satisfies WidgetRenderer;
