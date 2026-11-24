import WidgetState from '../../components/WidgetState';
import type { WidgetProps } from '../../dashboard/definitions';
import { familyHost } from '../../families/host/registry';
import type { FamilyViewModule } from '../../families/manifest.ts';
import SpecialProjectsWidget from './SpecialProjectsWidget';
import { specialProjectsDeclaration, specialProjectsProjection } from './specialProjectsProjection.ts';

const CURRENT: FamilyViewModule = {
  family: 'card',
  kind: 'special-projects',
  view: 'live',
  projection: specialProjectsProjection,
  declaration: specialProjectsDeclaration,
};

export default function SpecialProjectsView(props: WidgetProps) {
  if (props.widget.options?.view === 'history') return <SpecialProjectsWidget {...props} />;
  const Host = familyHost('card');
  return Host ? <Host {...props} spec={CURRENT} /> : <WidgetState kind="unavailable" message="Widget family host unavailable." />;
}
