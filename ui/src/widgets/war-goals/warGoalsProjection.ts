import { liveKey } from '../../charts/live.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { StatusMarkKind } from '../../draw/kit.ts';
import type { CardWidgetDeclaration } from '../../families/card/declaration.ts';
import { cardFlag, type CardItemRow, type CardModel, type CardProjection } from '../../families/card/model.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { knownValue, notRecorded, valueFromLive, type Value } from '../../families/value.ts';
import { formatDateHover, formatDateOnly } from '../../families/valueFormat.ts';
import type { WargoalEntry, WargoalJustification, WargoalRemoved, WargoalState, WargoalsReply } from '../../generated/dashboard';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { validateWargoalsReply } from './warGoalsData.ts';

export interface WarGoalsOptions {
  readonly targetNames: boolean;
}

const UNNAMED_GOAL = 'Not recorded';
const STATE: Readonly<Record<WargoalState, { readonly mark: StatusMarkKind; readonly label: string }>> = {
  atWar: { mark: 'running', label: 'At war' },
  ready: { mark: 'waiting', label: 'Ready' },
  inUse: { mark: 'running', label: 'In use' },
  expired: { mark: 'ended', label: 'Expired' },
};

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
  const country = (tag: string) => {
    const name = readView.who('country', tag).name;
    return name.status === 'ready' ? name.value : tag;
  };
  for (const tag of liberate) parts.push(`Liberate ${country(tag)}`);
  for (const tag of puppets) parts.push(`Puppet ${country(tag)}`);
  return parts.join(' · ');
}

function titled(readView: BoundedReadView, recipient: string, name: string, options: WarGoalsOptions): Pick<CardItemRow, 'flag' | 'title' | 'subtitle' | 'subject'> {
  const target = cardFlag(readView, recipient);
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
  return row.title.kind === 'known' ? String(row.title.value) : 'Kind not recorded';
}

function removedRow(readView: BoundedReadView, removed: WargoalRemoved, options: WarGoalsOptions): CardItemRow {
  const base = removed.recipient
    ? titled(readView, removed.recipient, removed.name ?? UNNAMED_GOAL, options)
    : { title: removed.name ? knownValue(removed.name) : notRecorded('The kind of war goal was not recorded') };
  const date = formatDateOnly(removed.removed);
  return {
    kind: 'item',
    key: `removed:${removed.handle}:${removed.removed}`,
    ...base,
    figure: { value: knownValue(removed.removedExact ? date : `By ${date}`), figure: 'Removed' },
    state: { mark: 'ended', label: 'Removed' },
    muted: true,
    tooltip: {
      title: tipTitle(base),
      hour: readView.hour,
      items: [
        ...grantItems(removed.granted, removed.grantedMissing),
        { label: removed.removedExact ? 'Removed' : 'Removed by', value: formatDateHover(removed.removed) },
        { label: 'Reason', missingReason: removed.reasonMissing },
      ],
    },
  };
}

function goalRow(reply: WargoalsReply, readView: BoundedReadView, goal: WargoalEntry, options: WarGoalsOptions): CardItemRow {
  const state = STATE[goal.state];
  let figure: CardItemRow['figure'];
  if (goal.state === 'ready') {
    figure = goal.expires === null
      ? { value: notRecorded('The war goal’s expiry was not recorded'), figure: 'Expiry' }
      : { value: knownValue(`Expires ${formatDateOnly(goal.expires)}`), figure: 'Expiry' };
  } else if (goal.state === 'expired' && goal.expires !== null) {
    figure = { value: knownValue(formatDateOnly(goal.expires)), figure: 'Expired' };
  }
  return {
    kind: 'item',
    key: `goal:${goal.handle}`,
    ...titled(readView, goal.recipient, goal.name, options),
    place: placeOf(reply, readView, goal.states, goal.liberate, goal.puppets),
    ...(figure ? { figure } : {}),
    state,
    ...(goal.state === 'expired' ? { muted: true } : {}),
    tooltip: {
      title: kindOf(goal.name).kind === 'known' ? goal.name : 'Kind not recorded',
      hour: readView.hour,
      items: [
        { label: 'State', value: state.label },
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
      ...(rows.length === 0 ? { empty: 'No war goals at this date.' } : {}),
      ...(reply.warnings.length ? { warnings: reply.warnings } : {}),
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
