import { createElement } from 'react';
import type { WidgetProps, WidgetRenderer } from '../../dashboard/definitions';
import CountryChip from '../../dashboard/CountryChip';
import PoliticalPowerWidget from './PoliticalPowerWidget';

export default { countryChip: CountryChip, render: (props: WidgetProps) => createElement(PoliticalPowerWidget, props) } satisfies WidgetRenderer;
