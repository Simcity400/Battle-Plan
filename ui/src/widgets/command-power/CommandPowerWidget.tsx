import type { WidgetProps } from '../../dashboard/definitions';
import DeclaredTimeChartWidget from '../../families/time-chart/DeclaredTimeChartWidget.tsx';
import { commandPowerDeclaration } from './commandPowerProjection.ts';

export default function CommandPowerWidget(props: WidgetProps) {
  return <DeclaredTimeChartWidget {...props} declaration={commandPowerDeclaration} />;
}
