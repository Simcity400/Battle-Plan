import { useCallback, useMemo, useState } from 'react';
import type { FamilyViewProps, FamilyViewRenderer } from '../../dashboard/definitions';
import { jumpItem } from '../../charts/jump.ts';
import type { InstalledAction } from '../../families/contract.ts';
import { FamilyWidget } from '../../families/host/registry.tsx';

function DivisionLogView(props: FamilyViewProps) {
  const track = props.runtime.tracks.find(item => item.id === props.widget.trackIds[0]);
  const scope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id]);
  const [groups, setGroups] = useState({ scope, expanded: '' });
  const expanded = groups.scope === scope ? groups.expanded : '';
  const options = useMemo(() => ({ ...props.options, expanded }), [props.options, expanded]);
  const toggle = useCallback(({ subject }: InstalledAction) => {
    if (!subject.startsWith('group:')) return;
    setGroups(previous => {
      const next = new Set(previous.scope === scope ? previous.expanded.split(' ').filter(Boolean) : []);
      if (!next.delete(subject)) next.add(subject);
      return { scope, expanded: [...next].join(' ') };
    });
  }, [scope]);
  return props.manifestDeclaration ? <FamilyWidget {...props}
    options={options}
    family={props.manifestDeclaration.family}
    declaration={props.manifestDeclaration}
    contextMenuItems={hit => track && typeof hit.meta === 'number' ? [jumpItem(props.runtime, track.id, hit.meta)] : []}
    onToggle={toggle}
    onActivate={toggle}
  /> : null;
}

export default { render: DivisionLogView } satisfies FamilyViewRenderer;
