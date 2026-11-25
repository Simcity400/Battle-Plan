import type { FamilyViewProps, FamilyViewRenderer } from '../../dashboard/definitions';
import { FamilyWidget } from '../../families/host/registry.tsx';

function EspionageLogView(props: FamilyViewProps) {
  return props.manifestDeclaration ? <FamilyWidget {...props} family="table" declaration={props.manifestDeclaration} /> : null;
}

export default { render: EspionageLogView } satisfies FamilyViewRenderer;
