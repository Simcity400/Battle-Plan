import WidgetState from '../../components/WidgetState';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { familyHost } from '../../families/host/registry';
import { OPTIONS } from './definition';
import { supremacySharesDeclaration, supremacyTimelineDeclaration } from './navalSupremacyProjection.ts';

export default function NavalSupremacyWidget(props: WidgetProps) {
  const timeline = resolveOptions(OPTIONS, props.widget.options).view === 'timeline';
  const declaration = timeline ? supremacyTimelineDeclaration : supremacySharesDeclaration;
  const Host = familyHost(declaration.family);
  if (!Host) return <WidgetState kind="unavailable" message={`Family host unavailable: ${declaration.family}`} />;
  return <Host key={declaration.family} {...props} spec={{ family: declaration.family, kind: declaration.kind, view: timeline ? 'timeline' : 'shares', projection: declaration.projection, declaration }} />;
}
