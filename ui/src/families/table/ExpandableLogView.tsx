import { useCallback, useMemo, useState } from 'react';
import type { FamilyViewProps, FamilyViewRenderer } from '../../dashboard/definitions';
import { jumpItem } from '../../charts/jump.ts';
import type { InstalledAction } from '../contract.ts';
import { FamilyWidget } from '../host/registry.tsx';

function ExpandableLogView(props: FamilyViewProps) {
  const track = props.runtime.tracks.find(item => item.id === props.widget.trackIds[0]);
  const scope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id]);
  const [groups, setGroups] = useState({ scope, expanded: '', full: '' });
  const current = groups.scope === scope ? groups : { scope, expanded: '', full: '' };
  const options = useMemo(() => ({ ...props.options, expanded: current.expanded, full: current.full }), [props.options, current.expanded, current.full]);
  const toggle = useCallback(({ subject }: InstalledAction) => {
    const all = subject.startsWith('all:group:');
    if (!all && !subject.startsWith('group:')) return;
    setGroups(previous => {
      const kept = previous.scope === scope ? previous : { scope, expanded: '', full: '' };
      const expanded = new Set(kept.expanded.split(' ').filter(Boolean));
      const full = new Set(kept.full.split(' ').filter(Boolean));
      const group = all ? subject.slice(4) : subject;
      if (all) {
        if (!full.delete(group)) full.add(group);
      } else if (!expanded.delete(group)) expanded.add(group);
      else full.delete(group);
      return { scope, expanded: [...expanded].join(' '), full: [...full].join(' ') };
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

export default { render: ExpandableLogView } satisfies FamilyViewRenderer;
