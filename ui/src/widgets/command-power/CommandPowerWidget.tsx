import type { WidgetProps } from '../../dashboard/definitions';
import { DeclaredFamilyWidget } from '../../families/host/index.ts';
import { timeChartHostAdapter } from '../../families/time-chart/hostAdapter.ts';
import { commandPowerDeclaration } from './commandPowerProjection.ts';

export default function CommandPowerWidget(props: WidgetProps) {
  return <DeclaredFamilyWidget {...props} adapter={timeChartHostAdapter} declaration={commandPowerDeclaration} />;
}
