import { createContext, useContext, useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import { createPortal } from 'react-dom';
import type { WidgetMeta, WidgetSkeletonKind } from './definitions';
import type { WidgetTime } from './model';

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
  const base = definition?.title ?? title ?? 'this widget';
  const lowered = PRESERVE_LEAD.test(base) ? base : base[0] ? base[0].toLowerCase() + base.slice(1) : base;
  const subject = definition?.subject ?? lowered;
  const things = definition?.things ?? subject;
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

export function useWidgetViewSwitch(spec: DynamicViewSwitch | null): void {
  const frame = useContext(WidgetFrameContext);
  const [owner] = useState(() => ({}));
  const signature = spec ? JSON.stringify([spec.label, spec.value, spec.options]) : '';
  useEffect(() => {
    if (!frame) return;
    frame.setDynamicViewSwitch(owner, spec);
    return () => frame.setDynamicViewSwitch(owner, null);
  }, [frame, owner, signature]);
}

export function WidgetCaption({ children, className = '' }: { children: ReactNode; className?: string }) {
  const frame = useContext(WidgetFrameContext);
  if (children === null || children === undefined || children === false) return null;
  const line = <div className={`dash-widget-caption${className ? ` ${className}` : ''}`}>{children}</div>;
  return frame?.captionSlot ? createPortal(line, frame.captionSlot) : line;
}

export interface HeaderBudgetFeatures {
  hasSummary: boolean;
  summaryChars?: number;
  hasViewSwitch: boolean;
  viewChoices?: number;
  viewChars?: number;
  viewMenuOnly?: boolean;
  hasSubject: boolean;
  subjectChars?: number;
  timeChars?: number;
  titleChars?: number;
}

export interface HeaderDropState {
  stage: 0 | 1 | 2 | 3 | 4;
  dropPill: boolean;
  collapseView: boolean;
  compactSubject: boolean;
  compactTime: boolean;
}

const CHAR_PX = 6.5;

export function headerDropStage(width: number | null, features: HeaderBudgetFeatures): HeaderDropState {
  if (width === null || !Number.isFinite(width)) {
    return { stage: 0, dropPill: false, collapseView: !!features.viewMenuOnly, compactSubject: false, compactTime: false };
  }
  const base = 76;
  const titleMin = Math.min(Math.max((features.titleChars ?? 14) * 7.5, 96), 175);
  const pillWidth = features.hasSummary ? Math.min(24, features.summaryChars ?? 12) * 6.5 + 22 : 0;
  const segmentedWidth = features.hasViewSwitch && !features.viewMenuOnly
    ? (features.viewChars ?? (features.viewChoices ?? 2) * 8) * CHAR_PX + (features.viewChoices ?? 2) * 20 + 10
    : 0;
  const viewMenuWidth = features.hasViewSwitch ? 88 : 0;
  const subjectFull = features.hasSubject ? Math.min(18, Math.max(12, features.subjectChars ?? 13)) * CHAR_PX + 52 : 0;
  const subjectCompact = features.hasSubject ? 48 : 0;
  const timeFull = Math.max(66, (features.timeChars ?? 4) * CHAR_PX + 40);
  const timeCompact = 32;
  const gap = 8;
  const rightCount = (features.hasSubject ? 1 : 0) + (features.hasViewSwitch ? 1 : 0) + 2;
  const gaps = (rightCount + (features.hasSummary ? 1 : 0) + 2) * gap;

  const fullNeed = base + gaps + titleMin + pillWidth + (features.viewMenuOnly ? viewMenuWidth : segmentedWidth) + subjectFull + timeFull;
  if (width >= fullNeed) {
    return { stage: 0, dropPill: false, collapseView: !!features.viewMenuOnly, compactSubject: false, compactTime: false };
  }
  const noPillNeed = base + gaps - (features.hasSummary ? gap : 0) + titleMin + (features.viewMenuOnly ? viewMenuWidth : segmentedWidth) + subjectFull + timeFull;
  if (width >= noPillNeed) {
    return { stage: 1, dropPill: true, collapseView: !!features.viewMenuOnly, compactSubject: false, compactTime: false };
  }
  const viewMenuNeed = base + gaps - (features.hasSummary ? gap : 0) + titleMin + viewMenuWidth + subjectFull + timeFull;
  if (width >= viewMenuNeed) {
    return { stage: 2, dropPill: true, collapseView: true, compactSubject: false, compactTime: false };
  }
  const compactSubjectNeed = base + gaps - (features.hasSummary ? gap : 0) + titleMin + viewMenuWidth + subjectCompact + timeFull;
  if (width >= compactSubjectNeed) {
    return { stage: 3, dropPill: true, collapseView: true, compactSubject: true, compactTime: false };
  }
  const compactTimeThreshold = base + gaps - (features.hasSummary ? gap : 0) + 72 + viewMenuWidth + subjectCompact + timeFull;
  if (width >= compactTimeThreshold) {
    return { stage: 3, dropPill: true, collapseView: true, compactSubject: true, compactTime: false };
  }
  const compactTime = width < base + gaps - (features.hasSummary ? gap : 0) + 72 + viewMenuWidth + subjectCompact + timeCompact + 28 || width < compactTimeThreshold;
  return { stage: 4, dropPill: true, collapseView: true, compactSubject: true, compactTime };
}
