import { useCallback, useMemo, useRef, useState } from 'react';
import type { WidgetProps, WidgetRenderer } from '../../dashboard/definitions';
import type { InstalledAction } from '../../families/contract.ts';
import { FamilyWidget } from '../../families/host/registry.tsx';
import LogKindFilter, { type LogKindFilterItem } from '../../families/table/LogKindFilter.tsx';
import { EVENT_KINDS } from './definition.ts';
import EventDetailWindow, { type EventDetailSubject } from './EventDetailWindow.tsx';
import { eventChoicesDeclaration } from './eventChoicesProjection.ts';

function EventChoicesView(props: WidgetProps) {
  const anchor = useRef<HTMLDivElement>(null);
  const track = props.runtime.tracks.find(item => item.id === props.widget.trackIds[0]);
  const scope = track ? `${track.source.id}:${track.source.revision}:${track.branch.id}` : '';
  const [opened, setOpened] = useState<{ readonly scope: string; readonly subject: EventDetailSubject } | null>(null);
  const shown = opened && opened.scope === scope ? opened.subject : null;

  const filterItems = useMemo<LogKindFilterItem[]>(() => EVENT_KINDS.map(kind => ({
    value: kind.value,
    label: kind.label,
    checked: props.widget.options?.[kind.value] !== false,
  })), [props.widget.options]);

  const onToggleFilter = useCallback((value: string, nextChecked: boolean) => {
    props.setOption?.(value, nextChecked);
  }, [props.setOption]);

  const onSelectAll = useCallback(() => {
    const next: Record<string, boolean> = {};
    for (const kind of EVENT_KINDS) next[kind.value] = true;
    if (props.setOptions) props.setOptions(next);
    else for (const [key, val] of Object.entries(next)) props.setOption?.(key, val);
  }, [props.setOption, props.setOptions]);

  const onActivate = useCallback((action: InstalledAction) => {
    const meta = action.meta as EventDetailSubject | null | undefined;
    if (!meta || typeof meta !== 'object' || typeof meta.key !== 'string') return;
    setOpened({ scope, subject: { ...meta, key: action.subject || meta.key, asOfHour: action.hour } });
  }, [scope]);

  return (
    <div ref={anchor} className="fh-detail-anchor">
      <LogKindFilter
        slot={props.filterSlot}
        allLabel="All events"
        noun="event types"
        items={filterItems}
        onToggle={onToggleFilter}
        onSelectAll={onSelectAll}
      />
      <FamilyWidget
        {...props}
        family="table"
        declaration={eventChoicesDeclaration}
        selectedKey={shown?.key ?? null}
        onActivate={onActivate}
      />
      <EventDetailWindow subject={shown} anchor={anchor} onClose={() => setOpened(null)} />
    </div>
  );
}

export default { render: EventChoicesView } satisfies WidgetRenderer;
