import { liveKey } from '../../charts/live.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { StatusMarkKind } from '../../draw/kit.ts';
import type { CardWidgetDeclaration } from '../../families/card/declaration.ts';
import { cardFlag, cardTitleText, type CardItemRow, type CardModel, type CardProjection } from '../../families/card/model.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { identityName } from '../../families/table/identityCell.ts';
import { knownValue, notRecorded, valueFromLive, type Value } from '../../families/value.ts';
import { formatDateHover, formatDateOnly } from '../../families/valueFormat.ts';
import type { WargoalEntry, WargoalJustification, WargoalRemoved, WargoalState, WargoalsReply } from '../../generated/dashboard';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { validateWargoalsReply } from './warGoalsData.ts';

export interface WarGoalsOptions {
  readonly targetNames: boolean;
}

const UNNAMED_GOAL = 'Not recorded';
const STATE: Readonly<Record<Exclude<WargoalState, 'unknown'>, { readonly mark: StatusMarkKind; readonly label: string }>> = {
  atWar: { mark: 'running', label: 'At war' },
  ready: { mark: 'waiting', label: 'Ready' },
  inUse: { mark: 'running', label: 'In use' },
  expired: { mark: 'ended', label: 'Expired' },
};
const WAR_UNREAD = 'Whether the two countries are at war was not recorded';

const dayCount = (days: number) => `${days} ${days === 1 ? 'day' : 'days'} left`;

function kindOf(name: string): Value<string> {
  return name === UNNAMED_GOAL || !name.trim() ? notRecorded('The kind of war goal was not recorded') : knownValue(name);
}

function placeOf(reply: WargoalsReply, readView: BoundedReadView, states: readonly number[], liberate: readonly string[], puppets: readonly string[]): string {
  const names = new Map(reply.stateNames.map(entry => [entry.id, entry]));
  const parts = states.map(id => {
    const entry = names.get(id);
    return entry?.known ? entry.name : 'State not recorded';
  });
  const country = (tag: string) => identityName(readView, 'country', tag);
  for (const tag of liberate) parts.push(`Liberate ${country(tag)}`);
  for (const tag of puppets) parts.push(`Puppet ${country(tag)}`);
  return parts.join(' · ');
}

function titled(readView: BoundedReadView, recipient: string, name: string, options: WarGoalsOptions, atHour?: number): Pick<CardItemRow, 'flag' | 'title' | 'subtitle' | 'subject'> {
  const target = cardFlag(readView, recipient, atHour);
  const kind = kindOf(name);
  if (!options.targetNames) return { flag: target.media, title: kind, subject: recipient };
  const kindText = kind.kind === 'known' ? kind.value : 'Kind not recorded';
  return { flag: target.media, title: target.name, subtitle: kindText, subject: recipient };
}

function justificationRow(reply: WargoalsReply, readView: BoundedReadView, entry: WargoalJustification, options: WarGoalsOptions): CardItemRow {
  const live = readView.live('main');
  const points = valueFromLive(live, liveKey(entry.id, 'points'));
  const total = Number(entry.total);
  const fraction: Value<number> = points.kind === 'known' || points.kind === 'zero' || points.kind === 'partial'
    ? (total > 0 ? knownValue(Math.max(0, Math.min(1, Number(points.value) / total))) : notRecorded('The justification cost was not recorded'))
    : notRecorded(points.kind === 'notRecorded' || points.kind === 'notApplicable' ? points.reason : 'Justification progress was not recorded');
  const days = Math.max(0, Math.ceil((entry.completion - readView.hour) / 24));
  const running = entry.paused === false;
  return {
    kind: 'item',
    key: `justify:${entry.id}`,
    ...titled(readView, entry.recipient, entry.name, options),
    place: placeOf(reply, readView, entry.states, [], []),
    meter: { fraction },
    figure: running
      ? { value: knownValue(dayCount(days)), figure: 'Days left' }
      : { value: notRecorded(entry.paused ? 'A paused justification has no completion date' : 'Whether the justification is paused was not recorded'), figure: 'Days left' },
    ...(entry.paused === null ? {} : { state: running ? { mark: 'running' as const, label: 'Justifying' } : { mark: 'waiting' as const, label: 'Paused' } }),
  };
}

const GRANT_SOURCE = 'War goal grant source was not recorded';

function grantItems(granted: number | null, grantedMissing: string | null): TooltipListItem[] {
  return [
    granted !== null ? { label: 'Granted', value: formatDateHover(granted) } : { label: 'Granted', missingReason: grantedMissing ?? 'The grant date was not recorded' },
    { label: 'Granted by', missingReason: GRANT_SOURCE },
  ];
}

function tipTitle(row: Pick<CardItemRow, 'title'>): string {
  return cardTitleText(row.title, 'Kind');
}

function removedRow(readView: BoundedReadView, removed: WargoalRemoved, options: WarGoalsOptions): CardItemRow {
  const base = removed.recipient
    ? titled(readView, removed.recipient, removed.name ?? UNNAMED_GOAL, options, removed.seen)
    : { title: removed.name ? knownValue(removed.name) : notRecorded('The kind of war goal was not recorded') };
  const targetItems: TooltipListItem[] = base.title.kind === 'pending'
    ? [{ label: 'Target', pendingReason: base.title.reason }]
    : [];
  return {
    kind: 'item',
    key: `removed:${removed.handle}:${removed.removed}`,
    ...base,
    figure: { value: knownValue(formatDateOnly(removed.removed)), figure: 'Removed' },
    state: { mark: 'ended', label: 'Removed' },
    muted: true,
    tooltip: {
      title: tipTitle(base),
      hour: readView.hour,
      items: [
        ...targetItems,
        ...grantItems(removed.granted, removed.grantedMissing),
        { label: 'Removed', value: formatDateHover(removed.removed) },
        { label: 'Reason', missingReason: removed.reasonMissing },
      ],
    },
  };
}

function goalRow(reply: WargoalsReply, readView: BoundedReadView, goal: WargoalEntry, options: WarGoalsOptions): CardItemRow {
  const state = goal.state === 'unknown' ? null : STATE[goal.state];
  let figure: CardItemRow['figure'];
  if (goal.state === 'unknown') {
    figure = { value: notRecorded(WAR_UNREAD), figure: 'State' };
  } else if (goal.state === 'ready') {
    figure = goal.expires === null
      ? { value: notRecorded('The war goal’s expiry was not recorded'), figure: 'Expiry' }
      : { value: knownValue(`Expires ${formatDateOnly(goal.expires)}`), figure: 'Expiry' };
  } else if (goal.state === 'expired' && goal.expires !== null) {
    figure = { value: knownValue(formatDateOnly(goal.expires)), figure: 'Expired' };
  }
  const head = titled(readView, goal.recipient, goal.name, options);
  const targetItems: TooltipListItem[] = head.title.kind === 'pending'
    ? [{ label: 'Target', pendingReason: head.title.reason }]
    : [];
  return {
    kind: 'item',
    key: `goal:${goal.handle}`,
    ...head,
    place: placeOf(reply, readView, goal.states, goal.liberate, goal.puppets),
    ...(figure ? { figure } : {}),
    ...(state ? { state } : {}),
    ...(goal.state === 'expired' ? { muted: true } : {}),
    tooltip: {
      title: kindOf(goal.name).kind === 'known' ? goal.name : 'Kind not recorded',
      hour: readView.hour,
      items: [
        ...targetItems,
        state ? { label: 'State', value: state.label } : { label: 'State', missingReason: WAR_UNREAD },
        ...(goal.expires !== null ? [{ label: 'Expires', value: formatDateHover(goal.expires) }] : []),
        ...grantItems(goal.granted, goal.grantedMissing),
      ],
    },
  };
}

export const warGoalsProjection: CardProjection<WarGoalsOptions> = registerFamilyProjection({
  family: 'card' as const,
  kind: 'war-goals',
  project(readView, options): CardModel {
    const hour = readView.hour;
    const columns = { flag: true, place: true, titleWidth: 152, titleWeight: 600, meter: 56, figure: 151, state: 107 };
    const raw = readView.panel<unknown>('main');
    if (raw === undefined) return { hour, rows: [], columns, rowHeight: 36 };
    const reply = validateWargoalsReply(raw);
    const rows: CardItemRow[] = [
      ...reply.justifications.map(entry => justificationRow(reply, readView, entry, options)),
      ...reply.goals.map(goal => goalRow(reply, readView, goal, options)),
      ...reply.removed.map(removed => removedRow(readView, removed, options)),
    ];
    return {
      hour,
      rows,
      columns,
      rowHeight: 36,
      ...(rows.length === 0 ? (reply.removedUnread !== null ? { empty: reply.removedUnread, emptyKind: 'unavailable' as const } : { empty: 'No war goals at this date.' }) : {}),
      ...(reply.warnings.length || (rows.length && reply.removedUnread !== null) ? { warnings: [...reply.warnings, ...(rows.length && reply.removedUnread !== null ? [reply.removedUnread] : [])] } : {}),
      nextChangeAt: (Math.floor(hour / 24) + 1) * 24,
    };
  },
}) as CardProjection<WarGoalsOptions>;

export const warGoalsDeclaration: CardWidgetDeclaration<WarGoalsOptions> = {
  family: 'card',
  kind: 'war-goals',
  label: 'War goals',
  subject: 'war goals',
  emptyCampaign: 'Load a campaign to see war goals.',
  icon: 'handshake',
  countries: { mode: 'single', choose: 'Choose a country to see its war goals.' },
  keyByBranch: true,
  jumpToDate: true,
  sources: ctx => ({
    owner: `war-goals:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${ctx.countryTag ?? ''}`,
    entries: ctx.countryTag
      ? [{
          name: 'main',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'point',
            channel: 'panel',
            coverage: { mode: 'keyed', keys: [ctx.countryTag] },
            product: 'wargoals',
            followQuery: { panel: 'wargoals', branchId: ctx.branch.id, country: countryFromKey(ctx.countryTag) },
            params: { branchId: ctx.branch.id, country: countryFromKey(ctx.countryTag) },
            validate: validateWargoalsReply,
          },
        }]
      : [],
  }),
  projection: warGoalsProjection,
  projectionOptions: ctx => ({ targetNames: ctx.options.targetNames === true }),
};
