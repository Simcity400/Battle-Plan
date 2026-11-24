import type { WidgetProps } from '../../dashboard/definitions';
import { DeclaredFamilyWidget } from '../../families/host/index.ts';
import { timeChartHostAdapter } from '../../families/time-chart/hostAdapter.ts';
import { politicalPowerDeclaration } from './politicalPowerProjection.ts';

export default function PoliticalPowerWidget(props: WidgetProps) {
  return <DeclaredFamilyWidget {...props} adapter={timeChartHostAdapter} declaration={politicalPowerDeclaration} />;
}
