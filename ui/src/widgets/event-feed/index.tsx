import { useCallback, useMemo, useRef, useState } from 'react';
import type { FamilyViewProps, FamilyViewRenderer } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model.ts';
import { useProduct } from '../../dashboard/products.ts';
import { failureText } from '../../dashboard/queryError.ts';
import { useCountryNameAt } from '../../dashboard/reachedProduct.ts';
import type { InstalledAction } from '../../families/contract.ts';
import { FamilyWidget } from '../../families/host/registry.tsx';
import { useExpandedGroups } from '../../families/table/ExpandableLogView.tsx';
import LogKindFilter, { type LogKindFilterItem } from '../../families/table/LogKindFilter.tsx';
import { PickerContext } from '../../families/table/picker.ts';
import EventDetailWindow from '../event-choices/EventDetailWindow.tsx';
import { readableRow, useEventDetail, useTooltipText } from '../event-choices/eventDetail.ts';
import { withHoverText } from './hoverText.ts';
import { CATEGORIES } from './definition.ts';
import EventFeedActivityStrip from './EventFeedActivityStrip.tsx';
import { feedKindItems, kindsRequest, validateKinds } from './eventFeedData.ts';
import type { FeedDetailSubject } from './eventFeedProjection.ts';

const isEventRow = (key: string): boolean => readableRow(key) && !key.startsWith('group:') && !key.startsWith('all:') && !key.startsWith('loading:');

function EventFeedView(props: FamilyViewProps) {
  const anchor = useRef<HTMLDivElement>(null);
  const track = props.runtime.tracks.find(item => item.id === props.widget.trackIds[0]);
  const scope = track ? `${track.source.id}:${track.source.revision}:${track.branch.id}` : '';
  const groups = useExpandedGroups(scope);
  const [opened, setOpened] = useState<{ readonly scope: string; readonly subject: FeedDetailSubject } | null>(null);
  const kinds = useProduct(props.runtime.client, track ? kindsRequest(track) : null, props.active, validateKinds, track?.source);
  const [frozenState, setFrozenState] = useState<{ readonly scope: string; readonly hour: number | null }>({ scope: '', hour: null });
  const [installedState, setInstalledState] = useState<{ readonly scope: string; readonly hour: number | null }>({ scope: '', hour: null });
  const onFrozenHour = useCallback((hour: number | null) => {
    setFrozenState(prev => (prev.scope === scope && prev.hour === hour ? prev : { scope, hour }));
  }, [scope]);
  const onInstalledHour = useCallback((hour: number | null) => {
    setInstalledState(prev => (prev.scope === scope && prev.hour === hour ? prev : { scope, hour }));
  }, [scope]);
  const shown = opened && opened.scope === scope ? opened.subject : null;
  const recipient = useCountryNameAt(props.runtime.client, track, shown?.countryTag ?? null, shown?.countryNameHour ?? null, shown !== null, { exact: true });
  const named = shown?.countryTag ? {
    ...shown,
    countryName: recipient.nameDetail ? `${recipient.name}. ${recipient.nameDetail}` : recipient.name,
  } : shown;
  const detail = useEventDetail(props.runtime, props.widget.trackIds[0], 'event-feed-tail', shown, shown !== null);
  const hover = useTooltipText(props.runtime, props.widget.trackIds[0], 'event-feed-tail', props.widget.time.mode === 'follow', props.active, isEventRow);

  const { options } = props;
  const familyOptions = useMemo(
    () => ({ ...withHoverText(options, hover.texts), expanded: groups.expanded, full: groups.full }),
    [options, hover.texts, groups.expanded, groups.full],
  );
  const recordingKinds = kinds.data;
  const categories = useMemo(
    () => CATEGORIES.filter(cat => options[cat.value] !== false).map(cat => cat.value),
    [options],
  );
  const excludedKinds = useMemo(() => {
    const raw = options.excludedKinds;
    if (typeof raw === 'string' && raw.length > 0) return raw.split(',').filter(Boolean);
    return [];
  }, [options.excludedKinds]);

  const filterItems = useMemo<LogKindFilterItem[]>(() => feedKindItems(recordingKinds ?? [], CATEGORIES, categories, excludedKinds, Infinity), [recordingKinds, categories, excludedKinds]);

  const onToggleFilter = useCallback((value: string, nextChecked: boolean) => {
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
    const meta = action.meta as FeedDetailSubject | null | undefined;
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
      <PickerContext.Provider value={(hour, _selected, children) => kinds.error
        ? children([], { kind: 'failed', message: failureText(kinds.error, 'Event kinds could not be loaded.') })
        : recordingKinds === undefined ? children([], { kind: 'loading' }) : children(feedKindItems(recordingKinds, CATEGORIES, categories, excludedKinds, hour))}><LogKindFilter
        shownHour={() => frozenHour ?? installedHour ?? pinnedHour ?? (track ? resolveWindow(props.widget, track, props.runtime.tracks, props.runtime.clock.getHour())?.lastHour ?? null : null)}
        slot={props.filterSlot}
        allLabel="All kinds"
        noun="kinds"
        items={filterItems}
        filtered={categories.length !== CATEGORIES.length || excludedKinds.length > 0}
        onToggle={onToggleFilter}
        onSelectAll={onSelectAll}
      /></PickerContext.Provider>
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
        beforeBody={showStrip ? (context, status) => <EventFeedActivityStrip
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
          context={context}
          refusal={status.kind === 'refused' ? status.reason : null}
        /> : undefined}
      /> : null}
      <EventDetailWindow subject={named} detail={detail} anchor={anchor} onClose={() => setOpened(null)} />
    </div>
  );
}

export default { render: EventFeedView } satisfies FamilyViewRenderer;
