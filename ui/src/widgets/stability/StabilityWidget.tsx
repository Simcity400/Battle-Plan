import type { FamilyViewProps } from '../../dashboard/definitions';
import { FamilyWidget } from '../../families/host/registry.tsx';
import StabilityDetail from './StabilityDetail.tsx';

export default function StabilityWidget(props: FamilyViewProps) {
  return (
    <StabilityDetail
      {...props}
      chart={onActivate => (props.manifestDeclaration ? (
        <FamilyWidget
          {...props}
          family={props.manifestDeclaration.family}
          declaration={props.manifestDeclaration}
          onActivate={action => onActivate(action.subject, action.hour)}
        />
      ) : null)}
    />
  );
}
