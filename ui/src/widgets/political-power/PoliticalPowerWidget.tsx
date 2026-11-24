import type { WidgetProps } from '../../dashboard/definitions';
import DeclaredTimeChartWidget from '../../families/time-chart/DeclaredTimeChartWidget.tsx';
import { politicalPowerDeclaration } from './politicalPowerProjection.ts';

export default function PoliticalPowerWidget(props: WidgetProps) {
  return <DeclaredTimeChartWidget {...props} declaration={politicalPowerDeclaration} />;
}
