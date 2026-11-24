import type { WidgetRenderer } from '../../dashboard/definitions';
import CountryChip from '../../dashboard/CountryChip';
import { useAirChoices } from '../../dashboard/useAirSuperiority';
import { airTimelineDeclaration } from './airTimelineProjection.ts';
import { OPTIONS } from './definition';

export default { declaration: airTimelineDeclaration, countryChip: CountryChip, useOptionChoices: (widget, runtime) => useAirChoices(widget, runtime, OPTIONS) } satisfies WidgetRenderer;
