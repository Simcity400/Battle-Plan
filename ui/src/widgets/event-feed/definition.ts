import type { IconName } from '../../components/Icons';
import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { LOG_OPTIONS } from '../../charts/log/logOptions.ts';
import type { EventFeedKind } from '../../generated/dashboard';

export const EVENT_ICONS: Record<EventFeedKind, IconName> = {
  warStarted: 'swords', warDeclared: 'swords', warJoined: 'swords', civilWarWarTransferred: 'flag-torn',
  annexed: 'flag-plus', warEnded: 'olive-branch', released: 'flag-plus', stateTransferred: 'map',
  peaceConferenceStarted: 'handshake', peace: 'olive-branch', capitulated: 'flag', ceasedToExist: 'flag-x',
  factionJoined: 'users-plus', factionLeft: 'users-minus', becameSubject: 'link', guaranteeGranted: 'shield-check',
  rulingPartyChanged: 'ballot', lawChanged: 'parliament', advisorHired: 'star', focusCompleted: 'target',
  projectFinished: 'flask', nukesIncreased: 'sparkle', nuclearStrike: 'sunburst',
  scriptEventFired: 'flag', newsEventFired: 'flag', eventOptionSelected: 'ballot',
  decisionSelected: 'ballot', decisionRemoved: 'ballot', decisionCancelled: 'ballot',
  decisionTimedOut: 'clock', decisionActivated: 'target', decisionCompleted: 'target', decisionReenabled: 'target',
  decisionRemovalEffect: 'ballot', decisionCancellationEffect: 'ballot', decisionCompletionEffect: 'target',
  decisionTimeoutEffect: 'clock', decisionDurationChanged: 'clock', eventWindowPresented: 'flag',
};

export const CATEGORIES = [
  { value: 'warsPeace', label: 'Wars & peace', palette: 1 },
  { value: 'diplomacy', label: 'Diplomacy', palette: 0 },
  { value: 'politics', label: 'Politics', palette: 5 },
  { value: 'milestones', label: 'Milestones', palette: 2 },
] as const;

export const OPTIONS: readonly OptionField[] = [
  ...CATEGORIES.map(category => ({ key: category.value, section: 'options' as const, label: category.label,
    kind: 'toggle' as const, default: true })),
  { key: 'artwork', section: 'options', label: 'Game artwork', kind: 'toggle', default: true },
  ...LOG_OPTIONS,
];

export const definition = {
  kind: 'event-feed', version: 1, title: 'Event feed', icon: 'clock',
  description: 'Key moments across every country.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 7, h: 14 },
  countries: { max: Number.MAX_SAFE_INTEGER, all: true }, options: OPTIONS,
} as const satisfies WidgetMeta;
