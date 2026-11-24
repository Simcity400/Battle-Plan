import { startUnseen } from '../../charts/timeline/runState.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { validateIdeaHistoryReply } from '../../dashboard/ideaHistoryReply.ts';
import type { CardWidgetDeclaration } from '../../families/card/declaration.ts';
import { cardMediaFromIcon, type CardItemRow, type CardModel, type CardProjection } from '../../families/card/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { knownValue, notRecorded } from '../../families/value.ts';
import { formatDateHover, formatDateOnly } from '../../families/valueFormat.ts';
import type { IdeaHistoryReply } from '../../generated/dashboard';
import { activeAt, spirits } from './spiritsData.ts';

export interface NationalSpiritsOptions {
  readonly branchFirst: number;
}

const COLUMNS = { art: { width: 22, height: 22 }, figure: 150, figureAlign: 'right' } as const;
const NO_ICON = 'The spirit has no icon in the current installation';
const replyLists = new WeakMap<IdeaHistoryReply, ReturnType<typeof spirits>>();

function listOf(reply: IdeaHistoryReply): ReturnType<typeof spirits> {
  let list = replyLists.get(reply);
  if (!list) replyLists.set(reply, list = spirits(reply));
  return list;
}

function nextBoundary(reply: IdeaHistoryReply, hour: number): number {
  let next = Infinity;
  for (const span of reply.spans) {
    if (span.firstHour > hour && span.firstHour < next) next = span.firstHour;
    if (span.lastHour + 1 > hour && span.lastHour + 1 < next) next = span.lastHour + 1;
  }
  for (const span of reply.unobserved) {
    if (span.firstHour > hour && span.firstHour < next) next = span.firstHour;
    if (span.lastHour + 1 > hour && span.lastHour + 1 < next) next = span.lastHour + 1;
  }
  return next;
}

export const nationalSpiritsProjection: CardProjection<NationalSpiritsOptions> = registerFamilyProjection({
  family: 'card' as const,
  kind: 'national-spirits',
  nextChange(readView) {
    const reply = readView.panel<IdeaHistoryReply>('main');
    return reply ? nextBoundary(reply, readView.hour) : Infinity;
  },
  project(readView, options): CardModel {
    const hour = readView.hour;
    const reply = readView.panel<IdeaHistoryReply>('main');
    if (!reply) return { hour, rows: [], columns: COLUMNS, rowHeight: 32 };
    const warnings = reply.warnings.length ? { warnings: reply.warnings } : {};
    if (reply.unobserved.some(span => span.firstHour <= hour && hour <= span.lastHour)) {
      return { hour, rows: [], columns: COLUMNS, rowHeight: 32, empty: 'National spirits were not recorded at this date.', emptyKind: 'unavailable', ...warnings };
    }
    const rows: CardItemRow[] = activeAt(listOf(reply), hour).map(spirit => {
      const span = spirit.spans.find(item => item.firstHour <= hour && hour <= item.lastHour)!;
      const before = startUnseen(span.firstHour, options.branchFirst, reply.unobserved);
      const since = before ? `Held before ${formatDateOnly(span.firstHour)}` : `Since ${formatDateOnly(span.firstHour)}`;
      const title = spirit.nameMissing ? notRecorded(spirit.nameMissing) : knownValue(spirit.name);
      const shown = spirit.nameMissing ? 'Name not recorded' : spirit.name;
      return {
        kind: 'item',
        key: spirit.token,
        art: cardMediaFromIcon(readView, spirit.icon, NO_ICON, spirit.nameMissing ? undefined : spirit.name),
        title,
        figure: { value: knownValue(since), figure: 'Since', muted: true },
        tooltip: {
          title: shown,
          fullText: shown,
          subtitle: 'National spirit',
          hour,
          items: [
            { label: before ? 'Held before' : 'Since', value: formatDateHover(span.firstHour) },
            ...(spirit.nameMissing ? [{ label: 'Name', missingReason: spirit.nameMissing }] : []),
            ...(spirit.icon ? [] : [{ label: 'Art', missingReason: `${NO_ICON}; art missing from the current installation` }]),
          ],
        },
      };
    });
    return {
      hour,
      rows,
      columns: COLUMNS,
      rowHeight: 32,
      ...(rows.length === 0 ? { empty: 'No national spirits at this date.' } : {}),
      ...warnings,
      nextChangeAt: nextBoundary(reply, hour),
    };
  },
}) as CardProjection<NationalSpiritsOptions>;

export const nationalSpiritsDeclaration: CardWidgetDeclaration<NationalSpiritsOptions> = {
  family: 'card',
  kind: 'national-spirits',
  label: 'National spirits',
  subject: 'national spirits',
  emptyCampaign: 'Load a campaign and choose a country to see its national spirits.',
  icon: 'star',
  countries: { mode: 'single', choose: 'Choose a country to see its national spirits.' },
  keyByBranch: true,
  jumpToDate: true,
  sources: ctx => ({
    owner: `national-spirits:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${ctx.countryTag ?? ''}`,
    entries: ctx.countryTag
      ? [{
          name: 'main',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'branch-to-hour',
            coverage: { mode: 'keyed', keys: [ctx.countryTag] },
            product: 'idea-history',
            followQuery: null,
            params: { branchId: ctx.branch.id, country: countryFromKey(ctx.countryTag) },
            buildSpanParams: base => base,
            validate: validateIdeaHistoryReply,
          },
        }]
      : [],
    allowEmptyFollow: true,
  }),
  projection: nationalSpiritsProjection,
  projectionOptions: ctx => ({ branchFirst: ctx.branch.firstHour }),
};
