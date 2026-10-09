import { createElement } from 'react';
import type { WidgetProps, WidgetRenderer } from '../../dashboard/definitions';
import MeasureCountryChip from '../country-measure/MeasureCountryChip';
import MeasureWidget from '../country-measure/MeasureWidget';
import type { MeasureConfig } from '../country-measure/MeasureWidget';

const config: MeasureConfig = { measure: 'politicalPower', title: 'Political power', noun: 'political power', unit: 'PP' };

export default { countryChip: MeasureCountryChip, render: (props: WidgetProps) => createElement(MeasureWidget, { ...props, config }) } satisfies WidgetRenderer;
