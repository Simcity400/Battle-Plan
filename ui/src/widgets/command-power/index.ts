import { createElement } from 'react';
import type { WidgetProps, WidgetRenderer } from '../../dashboard/definitions';
import MeasureWidget from '../country-measure/MeasureWidget';
import type { MeasureConfig } from '../country-measure/MeasureWidget';

const config: MeasureConfig = { measure: 'commandPower', title: 'Command power', noun: 'command power' };

export default { render: (props: WidgetProps) => createElement(MeasureWidget, { ...props, config }) } satisfies WidgetRenderer;
