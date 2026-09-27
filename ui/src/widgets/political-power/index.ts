import { createElement } from 'react';
import type { WidgetProps, WidgetRenderer } from '../../dashboard/definitions';
import MeasureWidget from '../country-measure/MeasureWidget';
import type { MeasureConfig } from '../country-measure/MeasureWidget';

const config: MeasureConfig = { measure: 'politicalPower', title: 'Political power', noun: 'political power' };

export default { render: (props: WidgetProps) => createElement(MeasureWidget, { ...props, config }) } satisfies WidgetRenderer;
