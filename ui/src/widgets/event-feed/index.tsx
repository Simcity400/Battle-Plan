import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { validateProductReply } from '../../dashboard/client.ts';
import type { FamilyViewProps, FamilyViewRenderer } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model.ts';
import type { InstalledAction } from '../../families/contract.ts';
import { FamilyWidget } from '../../families/host/registry.tsx';
import { useExpandedGroups } from '../../families/table/ExpandableLogView.tsx';
import LogKindFilter, { type LogKindFilterItem } from '../../families/table/LogKindFilter.tsx';
import type { EventFeedKindInfo } from '../../generated/dashboard';
import EventDetailWindow, { type EventDetailSubject } from '../event-choices/EventDetailWindow.tsx';
import { readableRow, useEventDetail, useTooltipText } from '../event-choices/eventDetail.ts';
import { withHoverText } from './hoverText.ts';
import { CATEGORIES } from './definition.ts';
import EventFeedActivityStrip from './EventFeedActivityStrip.tsx';
import { feedKindLabel, kindsRequest, validateKinds } from './eventFeedData.ts';

const CATEGORY_LABEL = new Map<string, string>(CATEGORIES.map(cat => [cat.value, cat.label]));
const isEventRow = (key: string): boolean => readableRow(key) && !key.startsWith('group:') && !key.startsWith('all:') && !key.startsWith('loading:');

function EventFeedView(props: FamilyViewProps) {
  const anchor = useRef<HTMLDivElement>(null);
  const track = props.runtime.tracks.find(item => item.id === props.widget.trackIds[0]);
  const scope = track ? `${track.source.id}:${track.source.revision}:${track.branch.id}` : '';
  const groups = useExpandedGroups(scope);
  const [opened, setOpened] = useState<{ readonly scope: string; readonly subject: EventDetailSubject } | null>(null);
  const [kinds, setKinds] = useState<{ readonly scope: string; readonly list: readonly EventFeedKindInfo[] } | null>(null);
  const [frozenState, setFrozenState] = useState<{ readonly scope: string; readonly hour: number | null }>({ scope: '', hour: null });
  const [installedState, setInstalledState] = useState<{ readonly scope: string; readonly hour: number | null }>({ scope: '', hour: null });
  const onFrozenHour = useCallback((hour: number | null) => {
    setFrozenState(prev => (prev.scope === scope && prev.hour === hour ? prev : { scope, hour }));
  }, [scope]);
  const onInstalledHour = useCallback((hour: number | null) => {
    setInstalledState(prev => (prev.scope === scope && prev.hour === hour ? prev : { scope, hour }));
  }, [scope]);
  const shown = opened && opened.scope === scope ? opened.subject : null;
  const detail = useEventDetail(props.runtime, props.widget.trackIds[0], 'event-feed-tail', shown, shown !== null);
  const hover = useTooltipText(props.runtime, props.widget.trackIds[0], 'event-feed-tail', props.widget.time.mode === 'follow', props.active, isEventRow);

  useEffect(() => {
    if (!track || !props.active) return;
    if (kinds?.scope === scope) return;
    const controller = new AbortController();
    const request = kindsRequest(track);
    props.runtime.client.product(request, controller.signal).then(reply => {
      if (controller.signal.aborted) return;
      setKinds({ scope, list: validateKinds(validateProductReply(reply, request).data) });
    }).catch(() => {});
    return () => controller.abort();
  }, [track, props.active, props.runtime.client, scope, kinds?.scope]);

  const { options } = props;
  const familyOptions = useMemo(
    () => ({ ...withHoverText(options, hover.texts), expanded: groups.expanded, full: groups.full }),
    [options, hover.texts, groups.expanded, groups.full],
  );
  const recordingKinds = kinds?.scope === scope ? kinds.list : null;
  const categories = useMemo(
    () => CATEGORIES.filter(cat => options[cat.value] !== false).map(cat => cat.value),
    [options],
  );
  const excludedKinds = useMemo(() => {
    const raw = options.excludedKinds;
    if (typeof raw === 'string' && raw.length > 0) return raw.split(',').filter(Boolean);
    return [];
  }, [options.excludedKinds]);

  const filterItems = useMemo<LogKindFilterItem[]>(() => {
    const excludedSet = new Set(excludedKinds);
    const categorySet = new Set<string>(categories);
    if (recordingKinds && recordingKinds.length > 0) {
      return recordingKinds
        .map(item => ({
          value: `kind:${item.kind}`,
          label: feedKindLabel(item.kind),
          group: CATEGORY_LABEL.get(item.category) ?? item.category,
          checked: categorySet.has(item.category) && !excludedSet.has(item.kind),
        }));
    }
    return CATEGORIES.map(cat => ({
      value: `cat:${cat.value}`,
      label: cat.label,
      checked: categorySet.has(cat.value),
    }));
  }, [recordingKinds, categories, excludedKinds]);

  const onToggleFilter = useCallback((value: string, nextChecked: boolean) => {
    if (value.startsWith('cat:')) {
      props.setOption?.(value.slice(4), nextChecked);
      return;
    }
    const kind = value.slice(5);
    const entry = recordingKinds?.find(item => item.kind === kind);
    const nextExcluded = new Set(excludedKinds);
    if (nextChecked) {
      nextExcluded.delete(kind);
      if (entry && options[entry.category] === false) {
        const encoded = [...nextExcluded].join(',');
        if (props.setOptions) props.setOptions({ [entry.category]: true, excludedKinds: encoded });
        else {
          props.setOption?.(entry.category, true);
          props.setOption?.('excludedKinds', encoded);
        }
        return;
      }
    } else {
      nextExcluded.add(kind);
    }
    props.setOption?.('excludedKinds', [...nextExcluded].join(','));
  }, [props.setOption, props.setOptions, options, recordingKinds, excludedKinds]);

  const onSelectAll = useCallback(() => {
    const next: Record<string, boolean | string> = { excludedKinds: '' };
    for (const cat of CATEGORIES) next[cat.value] = true;
    if (props.setOptions) props.setOptions(next);
    else for (const [key, val] of Object.entries(next)) props.setOption?.(key, val);
  }, [props.setOption, props.setOptions]);

  const onActivate = useCallback((action: InstalledAction) => {
    if (action.subject.startsWith('group:') || action.subject.startsWith('all:group:')) {
      groups.toggle(action);
      return;
    }
    const meta = action.meta as EventDetailSubject | null | undefined;
    if (!meta || typeof meta !== 'object' || typeof meta.key !== 'string') return;
    setOpened({ scope, subject: { ...meta, key: action.subject || meta.key, asOfHour: meta.asOfHour ?? action.hour } });
  }, [scope, groups.toggle]);

  const showStrip = options.activityStrip !== false;
  const following = props.widget.time.mode === 'follow';
  const win = track && !following ? resolveWindow(props.widget, track, [track], track.branch.lastHour) : null;
  const stripFrom = track && props.widget.time.mode === 'range' && win
    ? Math.max(Math.floor(win.firstHour), track.branch.firstHour)
    : null;
  const pinnedHour = track && !following && win ? Math.floor(win.lastHour) : null;
  const frozenHour = following && frozenState.scope === scope ? frozenState.hour : null;
  const installedHour = following && installedState.scope === scope ? installedState.hour : null;

  return (
    <div ref={anchor} className="fh-detail-anchor">
      <LogKindFilter
        slot={props.filterSlot}
        allLabel="All kinds"
        noun="kinds"
        items={filterItems}
        onToggle={onToggleFilter}
        onSelectAll={onSelectAll}
      />
      {showStrip ? (
        <EventFeedActivityStrip
          widget={props.widget}
          track={track}
          runtime={props.runtime}
          active={props.active}
          categories={categories}
          excludedKinds={excludedKinds}
          from={stripFrom}
          pinnedHour={pinnedHour}
          frozenHour={frozenHour}
          installedHour={installedHour}
        />
      ) : null}
      {props.manifestDeclaration ? <FamilyWidget
        {...props}
        options={familyOptions}
        onTooltipRow={hover.onTooltipRow}
        onFrozenHour={onFrozenHour}
        onInstalledHour={onInstalledHour}
        family={props.manifestDeclaration.family}
        declaration={props.manifestDeclaration}
        selectedKey={shown?.key ?? null}
        onToggle={groups.toggle}
        onActivate={onActivate}
      /> : null}
      <EventDetailWindow subject={shown} detail={detail} anchor={anchor} onClose={() => setOpened(null)} />
    </div>
  );
}

export default { render: EventFeedView } satisfies FamilyViewRenderer;
