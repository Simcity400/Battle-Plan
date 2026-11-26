import { useCallback, useMemo, useRef, useState } from 'react';
import type { FamilyViewProps, FamilyViewRenderer } from '../../dashboard/definitions';
import type { InstalledAction } from '../../families/contract.ts';
import { FamilyWidget } from '../../families/host/registry.tsx';
import LogKindFilter, { type LogKindFilterItem } from '../../families/table/LogKindFilter.tsx';
import { EVENT_KINDS } from './definition.ts';
import EventDetailWindow, { type EventDetailSubject } from './EventDetailWindow.tsx';
import { useEventDetail, useTooltipText } from './eventDetail.ts';
import { withHoverText } from '../event-feed/hoverText.ts';

function EventChoicesView(props: FamilyViewProps) {
  const anchor = useRef<HTMLDivElement>(null);
  const track = props.runtime.tracks.find(item => item.id === props.widget.trackIds[0]);
  const scope = track ? `${track.source.id}:${track.source.revision}:${track.branch.id}` : '';
  const [opened, setOpened] = useState<{ readonly scope: string; readonly subject: EventDetailSubject } | null>(null);
  const shown = opened && opened.scope === scope ? opened.subject : null;
  const detail = useEventDetail(props.runtime, props.widget.trackIds[0], 'event-choices-tail', shown, shown !== null);
  const hover = useTooltipText(props.runtime, props.widget.trackIds[0], 'event-choices-tail', props.widget.time.mode === 'follow', props.active);
  const options = useMemo(() => withHoverText(props.options, hover.texts), [props.options, hover.texts]);

  const filterItems = useMemo<LogKindFilterItem[]>(() => EVENT_KINDS.map(kind => ({
    value: kind.value,
    label: kind.label,
    checked: props.options[kind.value] !== false,
  })), [props.options]);

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
    setOpened({ scope, subject: { ...meta, key: action.subject || meta.key, asOfHour: meta.asOfHour ?? action.hour } });
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
      {props.manifestDeclaration ? <FamilyWidget
        {...props}
        options={options}
        onTooltipRow={hover.onTooltipRow}
        family={props.manifestDeclaration.family} declaration={props.manifestDeclaration}
        selectedKey={shown?.key ?? null}
        onActivate={onActivate}
      /> : null}
      <EventDetailWindow subject={shown} detail={detail} anchor={anchor} onClose={() => setOpened(null)} />
    </div>
  );
}

export default { render: EventChoicesView } satisfies FamilyViewRenderer;
