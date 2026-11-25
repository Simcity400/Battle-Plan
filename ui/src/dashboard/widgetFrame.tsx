import { createContext, useContext, useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import { createPortal } from 'react-dom';
import type { WidgetDefinition, WidgetMeta, WidgetSkeletonKind } from './definitions';
import type { WidgetSpec, WidgetTime } from './model';
import { resolveOptions } from './options';
import { resolveWidgetScopedOptions, resolveWidgetViewSwitch } from '../widgets/manifest';

export interface DynamicViewSwitch {
  label: string;
  value: string;
  options: readonly { value: string; label: string }[];
  onChange: (value: string) => void;
}

export interface WidgetFrameValue {
  title: string;
  subject: string;
  things: string;
  timeMode: WidgetTime['mode'];
  pinned: boolean;
  bounds: { firstHour: number; lastHour: number } | null;
  hasSubject: boolean;
  captionSlot: HTMLElement | null;
  openCountries?: () => void;
  openTime?: () => void;
  reportState: (owner: object, active: boolean) => void;
  announce: (key: string, text: string) => void;
  clearAnnouncement: (key: string) => void;
  setDynamicViewSwitch: (owner: object, spec: DynamicViewSwitch | null) => void;
  skeleton: WidgetSkeletonKind | (() => ReactNode);
  setSkeleton: (owner: object, skeleton: WidgetSkeletonKind | (() => ReactNode) | null) => void;
}

export const WidgetFrameContext = createContext<WidgetFrameValue | null>(null);

export function useWidgetFrame(): WidgetFrameValue | null {
  return useContext(WidgetFrameContext);
}

const PRESERVE_LEAD = /^(?:eAPM|PP|IC|VP|AI|DLC)\b/;

export function widgetWording(definition: Pick<WidgetMeta, 'title' | 'subject' | 'things'> | undefined, title?: string): { subject: string; things: string } {
  const customTitle = title && definition?.title && title !== definition.title;
  const base = (customTitle ? title : definition?.title ?? title) ?? 'this widget';
  const lowered = PRESERVE_LEAD.test(base) ? base : base[0] ? base[0].toLowerCase() + base.slice(1) : base;
  const subject = customTitle ? lowered : (definition?.subject ?? lowered);
  const things = customTitle ? lowered : (definition?.things ?? subject);
  return { subject, things };
}

export function capitaliseSubject(text: string): string {
  if (!text) return text;
  return PRESERVE_LEAD.test(text) ? text : text[0].toUpperCase() + text.slice(1);
}

export function useWidgetStatePresence(active: boolean): void {
  const frame = useContext(WidgetFrameContext);
  const [owner] = useState(() => ({}));
  useEffect(() => {
    if (!frame) return;
    frame.reportState(owner, active);
    return () => frame.reportState(owner, false);
  }, [frame, owner, active]);
}

export function useWidgetSkeleton(skeleton: WidgetSkeletonKind | (() => ReactNode) | null): void {
  const frame = useContext(WidgetFrameContext);
  const [owner] = useState(() => ({}));
  useEffect(() => {
    if (!frame) return;
    frame.setSkeleton(owner, skeleton);
    return () => frame.setSkeleton(owner, null);
  }, [frame, owner, skeleton]);
}

export function resolveDeclaredViewSwitch(
  definition: Pick<WidgetDefinition, 'options' | 'viewSwitch'> & Partial<Pick<WidgetDefinition, 'kind' | 'title'>> | undefined,
  widget: Pick<WidgetSpec, 'options' | 'countries' | 'time' | 'title'>,
  onChange: (option: string, value: string) => void,
): DynamicViewSwitch | null {
  const spec = definition?.kind ? resolveWidgetViewSwitch(definition.kind, definition as Pick<WidgetMeta, 'viewSwitch' | 'options' | 'title'>) : definition?.viewSwitch;
  const fields = definition?.kind ? resolveWidgetScopedOptions(definition.kind, widget.options, definition) : definition?.options;
  if (!spec || !fields?.length) return null;
  const resolved = resolveOptions(fields, widget.options);
  if (spec.when && !spec.when(resolved)) return null;
  const field = fields.find(item => item.key === spec.option);
  if (!field || field.kind !== 'choice') return null;
  if (field.visibleWhen && !field.visibleWhen(resolved, widget.countries, widget.time.mode)) return null;
  const options = spec.choices ?? field.choices;
  if (options.length <= 1) return null;
  return {
    label: spec.label ?? `${widget.title} ${field.label.toLowerCase()}`,
    value: String(resolved[spec.option] ?? field.default),
    options,
    onChange: value => onChange(spec.option, value),
  };
}

export function WidgetCaption({ children, className = '' }: { children: ReactNode; className?: string }) {
  const frame = useContext(WidgetFrameContext);
  if (children === null || children === undefined || children === false) return null;
  const line = <div className={`dash-widget-caption${className ? ` ${className}` : ''}`}>{children}</div>;
  return frame?.captionSlot ? createPortal(line, frame.captionSlot) : line;
}

export interface HeaderMeasuredWidths {
  fixed: number;
  titleFull: number;
  titleMin?: number;
  hasSummary: boolean;
  pill: number;
  hasViewSwitch: boolean;
  viewMenuOnly?: boolean;
  viewFull: number;
  viewMenu: number;
  hasSubject: boolean;
  subjectFull: number;
  subjectCompact: number;
  hasTime: boolean;
  timeFull: number;
  timeCompact: number;
  headerGap?: number;
  controlsGap?: number;
  editing?: boolean;
}

export interface HeaderDropState {
  stage: 0 | 1 | 2 | 3 | 4;
  dropPill: boolean;
  collapseView: boolean;
  compactSubject: boolean;
  compactTime: boolean;
}

export const FULL_HEADER_DROP: HeaderDropState = {
  stage: 0,
  dropPill: false,
  collapseView: false,
  compactSubject: false,
  compactTime: false,
};

export function headerDropStage(available: number | null, measured: HeaderMeasuredWidths): HeaderDropState {
  const menuOnly = !!measured.viewMenuOnly;
  if (available === null || !Number.isFinite(available) || available <= 0) {
    return { stage: 0, dropPill: false, collapseView: menuOnly, compactSubject: false, compactTime: false };
  }
  const hGap = measured.headerGap ?? 8;
  const cGap = measured.controlsGap ?? 6;
  const rightCount = (measured.hasSubject ? 1 : 0) + (measured.hasViewSwitch ? 1 : 0) + (measured.hasTime ? 1 : 0) + 1;
  const controlsGaps = Math.max(0, rightCount - 1) * cGap;
  const baseGaps = (measured.editing ? 2 : 1) * hGap + hGap + controlsGaps;
  const titleFull = Math.max(0, measured.titleFull);
  const titleMin = Math.min(titleFull, measured.titleMin ?? 64);
  const pill = measured.hasSummary ? Math.max(0, measured.pill) : 0;
  const viewFull = measured.hasViewSwitch ? Math.max(0, menuOnly ? measured.viewMenu : measured.viewFull) : 0;
  const viewMenu = measured.hasViewSwitch ? Math.max(0, measured.viewMenu) : 0;
  const subjectFull = measured.hasSubject ? Math.max(0, measured.subjectFull) : 0;
  const subjectCompact = measured.hasSubject ? Math.max(0, measured.subjectCompact) : 0;
  const timeFull = measured.hasTime ? Math.max(0, measured.timeFull) : 0;

  const base = measured.fixed + baseGaps;
  const stage0 = base + (measured.hasSummary ? hGap + pill : 0) + titleFull + viewFull + subjectFull + timeFull;
  if (stage0 <= available) {
    return { stage: 0, dropPill: false, collapseView: menuOnly, compactSubject: false, compactTime: false };
  }
  const stage1 = base + titleFull + viewFull + subjectFull + timeFull;
  if (measured.hasSummary && stage1 <= available) {
    return { stage: 1, dropPill: true, collapseView: menuOnly, compactSubject: false, compactTime: false };
  }
  const stage2 = base + titleFull + viewMenu + subjectFull + timeFull;
  if (measured.hasViewSwitch && !menuOnly && stage2 <= available) {
    return { stage: 2, dropPill: true, collapseView: true, compactSubject: false, compactTime: false };
  }
  const stage3 = base + titleMin + viewMenu + subjectCompact + timeFull;
  if (stage3 <= available) {
    return { stage: 3, dropPill: true, collapseView: true, compactSubject: true, compactTime: false };
  }
  return { stage: 4, dropPill: true, collapseView: true, compactSubject: true, compactTime: true };
}
