import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { validateProductReply } from '../../dashboard/client.ts';
import type { WidgetProps, WidgetRenderer } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model.ts';
import type { InstalledAction } from '../../families/contract.ts';
import { FamilyWidget } from '../../families/host/registry.tsx';
import LogKindFilter, { type LogKindFilterItem } from '../../families/table/LogKindFilter.tsx';
import type { FeedKindCode } from '../../generated/dashboard';
import EventDetailWindow, { type EventDetailSubject } from '../event-choices/EventDetailWindow.tsx';
import { CATEGORIES } from './definition.ts';
import EventFeedActivityStrip from './EventFeedActivityStrip.tsx';
import { EVENT_FEED_NOISE_KINDS, feedKindLabel, kindsRequest, validateSpan } from './eventFeedData.ts';
import { eventFeedDeclaration } from './eventFeedProjection.ts';

const CATEGORY_LABEL = new Map<string, string>(CATEGORIES.map(cat => [cat.value, cat.label]));

function EventFeedView(props: WidgetProps) {
  const anchor = useRef<HTMLDivElement>(null);
  const track = props.runtime.tracks.find(item => item.id === props.widget.trackIds[0]);
  const scope = track ? `${track.source.id}:${track.source.revision}:${track.branch.id}` : '';
  const [opened, setOpened] = useState<{ readonly scope: string; readonly subject: EventDetailSubject } | null>(null);
  const [kinds, setKinds] = useState<{ readonly scope: string; readonly list: readonly FeedKindCode[] } | null>(null);
  const shown = opened && opened.scope === scope ? opened.subject : null;

  useEffect(() => {
    if (!track || !props.active) return;
    if (kinds?.scope === scope) return;
    const controller = new AbortController();
    const request = kindsRequest(track);
    props.runtime.client.product(request, controller.signal).then(reply => {
      if (controller.signal.aborted) return;
      const span = validateSpan(validateProductReply(reply, request).data);
      setKinds({ scope, list: span.kinds });
    }).catch(() => {});
    return () => controller.abort();
  }, [track, props.active, props.runtime.client, scope, kinds?.scope]);

  const options = props.widget.options;
  const recordingKinds = kinds?.scope === scope ? kinds.list : null;
  const categories = useMemo(
    () => CATEGORIES.filter(cat => options?.[cat.value] !== false).map(cat => cat.value),
    [options],
  );
  const excludedKinds = useMemo(() => {
    const raw = options?.excludedKinds;
    if (typeof raw === 'string' && raw.length > 0) return raw.split(',').filter(Boolean);
    return [];
  }, [options?.excludedKinds]);

  const filterItems = useMemo<LogKindFilterItem[]>(() => {
    const excludedSet = new Set(excludedKinds);
    const categorySet = new Set<string>(categories);
    if (recordingKinds && recordingKinds.length > 0) {
      return recordingKinds
        .filter(item => !EVENT_FEED_NOISE_KINDS.has(item.kind))
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
      if (entry && options?.[entry.category] === false) {
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
    const meta = action.meta as EventDetailSubject | null | undefined;
    if (!meta || typeof meta !== 'object' || typeof meta.key !== 'string') return;
    setOpened({ scope, subject: { ...meta, key: action.subject || meta.key, asOfHour: action.hour } });
  }, [scope]);

  const showStrip = options?.activityStrip !== false;
  const following = props.widget.time.mode === 'follow';
  const win = track && !following ? resolveWindow(props.widget, track, [track], track.branch.lastHour) : null;
  const stripFrom = track && props.widget.time.mode === 'range' && win
    ? Math.max(Math.floor(win.firstHour), track.branch.firstHour)
    : null;
  const pinnedHour = track && !following && win ? Math.floor(win.lastHour) : null;

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
          kinds={recordingKinds}
          from={stripFrom}
          pinnedHour={pinnedHour}
        />
      ) : null}
      <FamilyWidget
        {...props}
        family="table"
        declaration={eventFeedDeclaration}
        selectedKey={shown?.key ?? null}
        onActivate={onActivate}
      />
      <EventDetailWindow subject={shown} anchor={anchor} onClose={() => setOpened(null)} />
    </div>
  );
}

export default { render: EventFeedView } satisfies WidgetRenderer;
