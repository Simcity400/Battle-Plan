import type { WidgetProps } from '../../dashboard/definitions';
import { DeclaredFamilyWidget } from '../../families/host/index.ts';
import { timeChartHostAdapter } from '../../families/time-chart/hostAdapter.ts';
import StabilityDetail from './StabilityDetail.tsx';
import { stabilityDeclaration } from './stabilityProjection.ts';

export default function StabilityWidget(props: WidgetProps) {
  return (
    <StabilityDetail
      {...props}
      chart={onActivate => (
        <DeclaredFamilyWidget
          {...props}
          adapter={timeChartHostAdapter}
          declaration={stabilityDeclaration}
          onActivate={action => onActivate(action.subject, action.hour)}
        />
      )}
    />
  );
}
