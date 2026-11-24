import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { CardWidgetDeclaration } from '../../families/card/declaration.ts';
import { cardMediaFromIcon, type CardHeroRow, type CardModel, type CardProjection, type CardRow } from '../../families/card/model.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { knownValue, notRecorded, type Value } from '../../families/value.ts';
import { formatDateHover, formatDateOnly } from '../../families/valueFormat.ts';
import type { FocusCardArt, FocusCardCurrent, FocusCardReply } from '../../generated/dashboard';

const COLUMNS = { date: true, art: { width: 30, height: 30 } } as const;

function nameOf(art: FocusCardArt): Value<string> {
  return art.name ? knownValue(art.name) : notRecorded(art.nameMissing ?? 'The focus name was not recorded');
}

function daysText(days: number): string {
  return Number.isInteger(days) ? String(days) : days.toFixed(2).replace(/0+$/, '').replace(/\.$/, '');
}

function heroRow(readView: BoundedReadView, current: FocusCardCurrent, hour: number): CardHeroRow {
  const title = nameOf(current.art);
  const art = cardMediaFromIcon(readView, current.art.icon, current.art.iconMissing, current.art.name ?? undefined);
  const base = { kind: 'hero' as const, key: `current:${current.art.token}`, art, title };
  if (current.continuous) return { ...base, state: { mark: current.paused ? 'waiting' : 'running', label: current.paused ? 'Paused' : 'Continuous focus' } };
  const parts: string[] = [];
  if (current.elapsedDays !== null && current.totalDays !== null) {
    parts.push(`Day ${Math.min(current.elapsedDays + 1, Math.ceil(current.totalDays))} of ${daysText(current.totalDays)}`);
  }
  if (current.daysLeft !== null) parts.push(`due ${formatDateOnly(hour + current.daysLeft * 24)}`);
  const figure = current.daysLeft !== null
    ? { value: knownValue(String(current.daysLeft)), caption: current.daysLeft === 1 ? 'Day left' : 'Days left' }
    : { value: notRecorded(current.progressMissing ?? 'Days left were not recorded'), caption: 'Days left' };
  const meter = current.permyriad !== null
    ? { fraction: current.permyriad === 0 ? { kind: 'zero' as const, value: 0 as const } : knownValue(current.permyriad / 10_000) }
    : { fraction: notRecorded(current.progressMissing ?? 'Progress was not recorded') };
  return {
    ...base,
    state: { mark: current.paused ? 'waiting' : 'running', label: current.paused ? 'Paused' : 'In progress' },
    ...(parts.length ? { detail: parts.join(' · ') } : {}),
    figure,
    meter,
  };
}

export const nationalFocusProjection: CardProjection<Record<string, never>> = registerFamilyProjection({
  family: 'card' as const,
  kind: 'national-focus',
  project(readView): CardModel {
    const hour = readView.hour;
    const reply = readView.panel<FocusCardReply>('main');
    if (!reply) return { hour, rows: [], columns: COLUMNS, rowHeight: 36 };
    if (reply.missing && !reply.current && reply.completed.length === 0) {
      return { hour, rows: [], columns: COLUMNS, rowHeight: 36, empty: reply.missing.reason, emptyKind: 'unavailable', ...(reply.warnings.length ? { warnings: reply.warnings } : {}) };
    }
    const rows: CardRow[] = [];
    if (reply.current) rows.push(heroRow(readView, reply.current, hour));
    const firstUndated = reply.completed.length - reply.undated;
    reply.completed.forEach((item, index) => {
      if (index === firstUndated && reply.undated > 0) rows.push({ kind: 'section', key: '__undated', label: 'Date not recorded', count: reply.undated });
      const title = nameOf(item.art);
      const shown = item.art.name ?? 'Name not recorded';
      rows.push({
        kind: 'item',
        key: `done:${index}:${item.art.token}`,
        date: { hour: item.completedByHour, ...(item.dateMissing ? { reason: item.dateMissing } : {}) },
        art: cardMediaFromIcon(readView, item.art.icon, item.art.iconMissing, item.art.name ?? undefined),
        title,
        tooltip: {
          title: shown,
          fullText: shown,
          hour,
          items: [
            item.completedByHour !== null
              ? { label: 'Completed by', value: formatDateHover(item.completedByHour) }
              : { label: 'Completed', missingReason: item.dateMissing ?? 'The completion date was not recorded' },
            ...(item.art.icon ? [] : [{ label: 'Art', missingReason: `${item.art.iconMissing ?? 'Icon not found'}; art missing from the current installation` }]),
          ],
        },
      });
    });
    return {
      hour,
      rows,
      columns: COLUMNS,
      rowHeight: 36,
      ...(rows.length === 0 ? { empty: 'No national focus at this date.' } : {}),
      ...(reply.warnings.length ? { warnings: reply.warnings } : {}),
    };
  },
}) as CardProjection<Record<string, never>>;

export const nationalFocusDeclaration: CardWidgetDeclaration<Record<string, never>> = {
  family: 'card',
  kind: 'national-focus',
  label: 'National focus',
  subject: 'national focus',
  emptyCampaign: 'Load a campaign to see national focuses.',
  icon: 'target',
  countries: { mode: 'single', choose: 'Choose a country to see its national focus.' },
  keyByBranch: true,
  jumpToDate: true,
  sources: ctx => ({
    owner: `national-focus:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${ctx.countryTag ?? ''}`,
    entries: ctx.countryTag
      ? [{
          name: 'main',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'point',
            channel: 'followed',
            coverage: { mode: 'keyed', keys: [ctx.countryTag] },
            product: 'focus-card',
            followQuery: null,
            params: { branchId: ctx.branch.id, country: countryFromKey(ctx.countryTag) },
            provenInterval: data => {
              const reply = data as FocusCardReply | undefined;
              return reply ? [reply.hour, reply.validThrough] : null;
            },
          },
        }]
      : [],
    allowEmptyFollow: true,
  }),
  projection: nationalFocusProjection,
  projectionOptions: () => ({}),
};
