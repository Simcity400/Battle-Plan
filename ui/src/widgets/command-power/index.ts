import { createElement } from 'react';
import type { WidgetProps, WidgetRenderer } from '../../dashboard/definitions';
import CountryChip from '../../dashboard/CountryChip';
import MeasureWidget from '../country-measure/MeasureWidget';
import type { MeasureConfig } from '../country-measure/MeasureWidget';
import { definition } from './definition';

const config: MeasureConfig = { measure: 'commandPower', title: 'Command power', noun: 'command power', unit: 'CP', options: definition.options, ceiling: 'maximum command power' };

export default { countryChip: CountryChip, render: (props: WidgetProps) => createElement(MeasureWidget, { ...props, config }) } satisfies WidgetRenderer;
