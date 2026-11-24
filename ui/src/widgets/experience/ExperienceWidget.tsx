import type { WidgetProps } from '../../dashboard/definitions';
import DeclaredTimeChartWidget from '../../families/time-chart/DeclaredTimeChartWidget.tsx';
import { experienceDeclaration } from './experienceProjection.ts';

export default function ExperienceWidget(props: WidgetProps) {
  return <DeclaredTimeChartWidget {...props} declaration={experienceDeclaration} />;
}
