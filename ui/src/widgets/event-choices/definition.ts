import type { WidgetMeta } from '../../dashboard/definitions';
import type { EventFeedKind } from '../../generated/dashboard';
import type { OptionField } from '../../dashboard/options';
import { LOG_OPTIONS } from '../../charts/log/logOptions.ts';

/** The event kinds a card can open with: each a toggle and a filter chip. */
export const EVENT_KINDS: readonly { value: string; label: string; kind: EventFeedKind }[] = [
  { value: 'countryEvents', label: 'Country events', kind: 'scriptEventFired' },
  { value: 'newsEvents', label: 'News events', kind: 'newsEventFired' },
];
export const AWAITING = { value: 'awaiting', label: 'Awaiting choice only', hint: 'Only events still awaiting a choice at the moment shown.' } as const;

export const OPTIONS: readonly OptionField[] = [
  ...EVENT_KINDS.map(kind => ({ key: kind.value, section: 'options' as const, label: kind.label, kind: 'toggle' as const, default: true })),
  { key: AWAITING.value, section: 'options', label: AWAITING.label, hint: AWAITING.hint, kind: 'toggle', default: false },
  { key: 'artwork', section: 'options', label: 'Game artwork', kind: 'toggle', default: true },
  ...LOG_OPTIONS,
];

export const definition = {
  kind: 'event-choices', version: 1, title: 'Event choices', icon: 'ballot', category: 'Politics',
  description: 'Events each country received, the option it chose and how long it took.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 6, h: 8 }, defaultSize: { w: 12, h: 12 },
  countries: { max: Number.MAX_SAFE_INTEGER, all: true }, options: OPTIONS,
} as const satisfies WidgetMeta;
