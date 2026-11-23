import { useContext, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Icon } from '../components/Icons';
import { StyledIcon } from '../shell/appearance/StyledIcon';
import { WidgetIconsContext } from '../shell/appearance/WidgetIconMenu';
import type { MapOptions } from '../shell/api';
import type { DashboardSource } from './client';
import type { WidgetDefinition, WidgetRuntime } from './definitions';
import type { BoundTrack, DashboardTemplate, WidgetSpec } from './model';
import { recordingDisplayName } from './recordingNames';
import DefaultCountryChip from './CountryChip';
import WidgetCountryControl from './WidgetCountryControl';
import WidgetSettingsMenu from './WidgetSettingsMenu';
import WidgetTimeChip, { timeChipSummary } from './WidgetTimeChip';
import WidgetViewSwitch from './WidgetViewSwitch';
import { FULL_HEADER_DROP, headerDropStage } from './widgetFrame';
import type { DynamicViewSwitch, HeaderDropState } from './widgetFrame';

export interface WidgetHeaderProps {
  widget: WidgetSpec;
  definition: WidgetDefinition | undefined;
  effective: WidgetSpec;
  campaignTrack: BoundTrack | undefined;
  runtime: WidgetRuntime;
  template?: DashboardTemplate;
  editing?: boolean;
  readOnly?: boolean;
  active?: boolean;
  activeSwitch: DynamicViewSwitch | null;
  hasBodyState: boolean;
  readingBadge?: string | null;
  summarySlotRef: (element: HTMLSpanElement | null) => void;
  countrySlotRef: (element: HTMLSpanElement | null) => void;
  headerSlotElement: HTMLSpanElement | null;
  source?: DashboardSource;
  onApply?: (widget: WidgetSpec, view?: boolean) => void;
  onRemove?: () => void;
  onReset?: () => void;
  resetUndoVisible?: boolean;
  onUndoReset?: () => void;
  mapOptions?: MapOptions;
  onMapOptions?: (options: MapOptions) => void;
  onRegisterOpenCountries?: (open: (() => void) | undefined) => void;
  onRegisterOpenTime?: (open: (() => void) | undefined) => void;
}

const noop = () => {};

export default function WidgetHeader({
  widget,
  definition,
  effective,
  campaignTrack,
  runtime,
  template,
  editing = false,
  readOnly = false,
  active = false,
  activeSwitch,
  hasBodyState,
  readingBadge = null,
  summarySlotRef,
  countrySlotRef,
  headerSlotElement,
  source,
  onApply,
  onRemove,
  onReset,
  resetUndoVisible = false,
  onUndoReset,
  mapOptions,
  onMapOptions,
  onRegisterOpenCountries,
  onRegisterOpenTime,
}: WidgetHeaderProps) {
  const headerRef = useRef<HTMLElement>(null);
  const titleRef = useRef<HTMLHeadingElement>(null);
  const [headerWidth, setHeaderWidth] = useState<number | null>(null);
  const [titleClipped, setTitleClipped] = useState(false);
  const [hasSummaryContent, setHasSummaryContent] = useState(false);
  const [measureEpoch, setMeasureEpoch] = useState(0);
  const [drop, setDrop] = useState<HeaderDropState>(FULL_HEADER_DROP);
  const pendingMeasure = useRef(false);
  const measuredHeader = useRef({
    measuredOnce: false,
    lastHeaderWidth: 0,
    probeKey: '',
    pillKey: '', pill: 0,
    viewFullKey: '', viewFull: 0,
    viewMenuKey: '', viewMenu: 0,
    subjectFullKey: '', subjectFull: 0,
    subjectCompact: 0,
    timeFullKey: '', timeFull: 0,
    timeCompact: 0,
  });

  const icons = useContext(WidgetIconsContext);
  const icon = icons.style(widget.kind);
  const CountryChip = definition?.countryChip ?? DefaultCountryChip;
  const viewMenuOnly = (activeSwitch?.options.length ?? 0) >= 4;
  const viewKey = activeSwitch ? `${activeSwitch.options.map(item => item.label).join('|')}:${activeSwitch.value}` : '';
  const hasSubject = definition?.data !== false && definition?.countrySelection !== false;
  const showTime = definition?.data !== false && (definition?.timeModes.length ?? 0) > 0;
  const timeInfo = showTime ? timeChipSummary(widget, effective, campaignTrack, runtime.tracks, runtime.clock.getHour?.() ?? 0) : null;
  const timeKey = showTime ? (widget.time.mode === 'follow' ? 'follow' : (timeInfo?.label ?? '')) : '';
  const subjectKey = hasSubject ? `${JSON.stringify(widget.countries)}:${campaignTrack?.id ?? ''}:${campaignTrack?.selectedTag ?? ''}` : '';
  const hasSummary = hasSummaryContent && !hasBodyState;
  const summaryKey = hasSummary ? '1' : '';
  const effectiveDrop: HeaderDropState = viewMenuOnly && !drop.collapseView
    ? { ...drop, collapseView: true }
    : drop;

  useEffect(() => {
    if (!headerSlotElement || typeof MutationObserver === 'undefined') {
      setHasSummaryContent(false);
      return;
    }
    const inspect = () => {
      const next = headerSlotElement.childNodes.length > 0;
      setHasSummaryContent(previous => previous === next ? previous : next);
    };
    inspect();
    const observer = new MutationObserver(inspect);
    observer.observe(headerSlotElement, { childList: true });
    return () => observer.disconnect();
  }, [headerSlotElement]);

  useEffect(() => {
    const header = headerRef.current;
    if (!header || typeof ResizeObserver === 'undefined') return;
    const isPlaying = () => !readOnly && (runtime.clock.getSnapshot?.().playing ?? false);
    const sizes = new WeakMap<Element, number>();
    const remember = (el: Element | null) => {
      if (!el) return;
      sizes.set(el, el === header ? Math.floor(header.clientWidth) : Math.ceil(el.getBoundingClientRect().width));
    };
    const trigger = (headerChanged: boolean, slotChanged: boolean) => {
      if (!headerChanged && !slotChanged) return;
      if (headerChanged) {
        const next = Math.floor(header.clientWidth);
        setHeaderWidth(previous => previous === next ? previous : next);
      }
      if (slotChanged) {
        if (isPlaying()) pendingMeasure.current = true;
        else setMeasureEpoch(previous => previous + 1);
      }
    };
    const observer = new ResizeObserver(entries => {
      let headerChanged = false;
      let slotChanged = false;
      for (const entry of entries) {
        const el = entry.target;
        const next = el === header ? Math.floor(header.clientWidth) : Math.ceil(el.getBoundingClientRect().width);
        const previous = sizes.get(el);
        if (previous === next) continue;
        sizes.set(el, next);
        if (el === header) headerChanged = true;
        else slotChanged = true;
      }
      trigger(headerChanged, slotChanged);
    });
    const observe = (el: Element | null) => {
      if (!el) return;
      remember(el);
      observer.observe(el);
    };
    observe(header);
    if (hasSummary && !effectiveDrop.dropPill) observe(header.querySelector('.dash-widget-summary'));
    if (hasSubject) observe(header.querySelector('.dash-widget-country-button, .dash-widget-controls > .dash-widget-country, .dash-widget-controls > .dash-widget-country-slot'));
    if (activeSwitch) observe(header.querySelector('[data-view-switch]'));
    if (showTime) observe(header.querySelector('.dash-time-chip'));
    const next = Math.floor(header.clientWidth);
    setHeaderWidth(previous => previous === next ? previous : next);
    if (isPlaying()) pendingMeasure.current = true;
    const unsub = !readOnly ? runtime.clock.subscribe?.(() => {
      if (isPlaying() || !pendingMeasure.current) return;
      pendingMeasure.current = false;
      const next = Math.floor(header.clientWidth);
      setHeaderWidth(previous => previous === next ? previous : next);
      setMeasureEpoch(previous => previous + 1);
    }) : undefined;
    return () => { observer.disconnect(); unsub?.(); };
  }, [readOnly, runtime.clock, hasSummary, effectiveDrop.dropPill, effectiveDrop.collapseView, effectiveDrop.compactSubject, effectiveDrop.compactTime, !!activeSwitch, hasSubject, showTime, subjectKey]);

  useLayoutEffect(() => {
    const header = headerRef.current;
    if (!header || header.clientWidth <= 0) return;
    const playing = !readOnly && (runtime.clock.getSnapshot?.().playing ?? false);
    const cache = measuredHeader.current;
    const headerResized = cache.measuredOnce && header.clientWidth !== cache.lastHeaderWidth;
    if (playing && cache.measuredOnce && !headerResized) {
      pendingMeasure.current = true;
      return;
    }
    cache.measuredOnce = true;
    cache.lastHeaderWidth = header.clientWidth;
    const available = header.clientWidth - 16;
    const widthOf = (el: HTMLElement | null) => el ? Math.ceil(el.getBoundingClientRect().width) : 0;
    const iconEl = header.querySelector<HTMLElement>('.dash-widget-icon');
    const gripEl = header.querySelector<HTMLElement>('.dash-drag-grip');
    const settingsEl = header.querySelector<HTMLElement>('[data-widget-settings]');
    const summaryEl = header.querySelector<HTMLElement>('.dash-widget-summary');
    const subjectEl = header.querySelector<HTMLElement>('.dash-widget-country-button, .dash-widget-controls > .dash-widget-country, .dash-widget-controls > .dash-widget-country-slot');
    const viewEl = header.querySelector<HTMLElement>('[data-view-switch]');
    const timeEl = header.querySelector<HTMLElement>('.dash-time-chip');
    const heading = titleRef.current;
    const titleFull = heading ? Math.ceil(heading.scrollWidth) : 0;
    const fixed = widthOf(iconEl) + widthOf(settingsEl) + widthOf(gripEl);

    if (hasSummary && !effectiveDrop.dropPill) {
      cache.pill = summaryEl ? Math.ceil(summaryEl.scrollWidth) : 0;
      cache.pillKey = summaryKey;
    }
    if (activeSwitch) {
      if (!effectiveDrop.collapseView) {
        cache.viewFull = widthOf(viewEl);
        cache.viewFullKey = viewKey;
      } else {
        cache.viewMenu = widthOf(viewEl);
        cache.viewMenuKey = viewKey;
      }
    }
    if (hasSubject) {
      if (!effectiveDrop.compactSubject) {
        cache.subjectFull = widthOf(subjectEl);
        cache.subjectFullKey = subjectKey;
      } else {
        cache.subjectCompact = widthOf(subjectEl);
      }
    }
    if (showTime) {
      if (!effectiveDrop.compactTime) {
        cache.timeFull = widthOf(timeEl);
        cache.timeFullKey = timeKey;
      } else {
        cache.timeCompact = widthOf(timeEl);
      }
    }

    const rawNeedsFull = (hasSummary && cache.pillKey !== summaryKey && effectiveDrop.dropPill)
      || (!!activeSwitch && !viewMenuOnly && cache.viewFullKey !== viewKey && effectiveDrop.collapseView)
      || (hasSubject && cache.subjectFullKey !== subjectKey && effectiveDrop.compactSubject)
      || (showTime && cache.timeFullKey !== timeKey && effectiveDrop.compactTime);
    const probeKey = `${summaryKey}\u0000${viewKey}\u0000${subjectKey}\u0000${timeKey}`;
    const needsFullMeasure = rawNeedsFull && cache.probeKey !== probeKey;
    if (needsFullMeasure) cache.probeKey = probeKey;

    const subjectPresent = hasSubject && (effectiveDrop.compactSubject ? !!subjectEl : cache.subjectFull > 0);
    const timePresent = showTime && (effectiveDrop.compactTime ? !!timeEl : cache.timeFull > 0);
    const viewPresent = !!activeSwitch && (effectiveDrop.collapseView ? !!viewEl : cache.viewFull > 0);

    const next: HeaderDropState = needsFullMeasure
      ? { stage: 0, dropPill: false, collapseView: viewMenuOnly, compactSubject: false, compactTime: false }
      : headerDropStage(available, {
          fixed,
          titleFull,
          hasSummary,
          pill: cache.pillKey === summaryKey ? cache.pill : 0,
          hasViewSwitch: viewPresent,
          viewMenuOnly,
          viewFull: cache.viewFullKey === viewKey ? cache.viewFull : 0,
          viewMenu: cache.viewMenuKey === viewKey && cache.viewMenu > 0 ? cache.viewMenu : 84,
          hasSubject: subjectPresent,
          subjectFull: cache.subjectFullKey === subjectKey ? cache.subjectFull : 0,
          subjectCompact: cache.subjectCompact > 0 ? cache.subjectCompact : 42,
          hasTime: timePresent,
          timeFull: cache.timeFullKey === timeKey ? cache.timeFull : 0,
          timeCompact: cache.timeCompact > 0 ? cache.timeCompact : 24,
          editing,
        });

    setDrop(previous => previous.stage === next.stage && previous.dropPill === next.dropPill
      && previous.collapseView === next.collapseView && previous.compactSubject === next.compactSubject
      && previous.compactTime === next.compactTime ? previous : next);
  }, [headerWidth, measureEpoch, readOnly, runtime.clock, widget.title, summaryKey, hasSummary, subjectKey, hasSubject, activeSwitch, viewKey, viewMenuOnly, showTime, timeKey, editing,
    effectiveDrop.dropPill, effectiveDrop.collapseView, effectiveDrop.compactSubject, effectiveDrop.compactTime]);

  useLayoutEffect(() => {
    const heading = titleRef.current;
    if (!heading) return;
    const playing = !readOnly && (runtime.clock.getSnapshot?.().playing ?? false);
    if (playing) return;
    const clipped = heading.scrollWidth > heading.clientWidth;
    setTitleClipped(previous => previous === clipped ? previous : clipped);
  }, [headerWidth, measureEpoch, readOnly, runtime.clock, widget.title, effectiveDrop.stage]);

  return <header ref={headerRef} className="dash-widget-header" data-header-stage={effectiveDrop.stage}>
    {editing && !readOnly && <span className="dash-drag-grip" data-tooltip="Drag to move" aria-hidden="true"><Icon name="grip" /></span>}
    <span className="dash-widget-icon" aria-hidden="true" data-widget-icon={readOnly ? undefined : widget.id} style={icon.colour ? { color: icon.colour } : undefined}
      onContextMenu={readOnly ? undefined : event => {
        if (!definition) return;
        event.preventDefault();
        icons.openMenu(widget.kind, event.currentTarget, definition.title, event.clientX, event.clientY);
      }}><StyledIcon icon={icon.icon} fallback={definition?.icon ?? 'grid'} /></span>
    <div className="dash-widget-title">
      <h2 ref={titleRef} data-tooltip-overflow={widget.title} tabIndex={!readOnly && titleClipped ? 0 : undefined}>{widget.title}</h2>
      {source && <span className="dash-widget-subtitle">{recordingDisplayName(source.path)}</span>}
    </div>
    <span className="dash-widget-summary" ref={summarySlotRef} hidden={effectiveDrop.dropPill || hasBodyState || undefined} />
    {readingBadge && <span className="dash-reading-badge is-in-header">{readingBadge}</span>}
    <div className="dash-widget-controls">
      {hasSubject && (readOnly || !template || !onApply
        ? <CountryChip widget={widget} runtime={runtime} active={false} compact={effectiveDrop.compactSubject} onSlot={countrySlotRef} />
        : <WidgetCountryControl widget={widget} template={template} runtime={runtime} onApply={onApply}
            compact={effectiveDrop.compactSubject} onRegisterOpen={onRegisterOpenCountries}>
            <CountryChip widget={widget} runtime={runtime} active={active} compact={effectiveDrop.compactSubject} onSlot={countrySlotRef} />
          </WidgetCountryControl>)}
      {activeSwitch && <WidgetViewSwitch label={activeSwitch.label} value={activeSwitch.value} options={activeSwitch.options}
        compact={effectiveDrop.collapseView} readOnly={readOnly} onChange={readOnly ? noop : activeSwitch.onChange} />}
      {showTime && (readOnly || !onApply
        ? timeInfo && <span className={`dash-time-chip is-static${effectiveDrop.compactTime ? ' is-compact' : ''}`} data-time-mode={timeInfo.mode}>
            {timeInfo.mode === 'follow' ? <span className="dash-time-dot" aria-hidden="true" />
              : timeInfo.mode === 'pin' ? <span className="dash-time-dot is-pinned" aria-hidden="true" />
              : <Icon name="clock" className="dash-time-icon" />}
            {!effectiveDrop.compactTime && <span className="dash-time-label">{timeInfo.label}</span>}
          </span>
        : <WidgetTimeChip widget={widget} effective={effective} reference={campaignTrack} runtime={runtime}
            modes={definition!.timeModes} compact={effectiveDrop.compactTime}
            onChange={time => onApply({ ...widget, time }, true)} onRegisterOpen={onRegisterOpenTime} />)}
      {!readOnly && template && onApply && <WidgetSettingsMenu widget={widget} template={template} runtime={runtime} onApply={onApply}
        onRemove={onRemove} onReset={onReset}
        resetUndoVisible={resetUndoVisible} onUndoReset={onUndoReset}
        mapOptions={mapOptions} onMapOptions={onMapOptions} />}
    </div>
  </header>;
}
