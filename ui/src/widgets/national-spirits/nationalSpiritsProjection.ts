import { countryFromKey } from '../../dashboard/countryKey.ts';
import { holdingLabel, holdingSince, ideaHistoryProof, ideaNameMissing, lookOf, validateIdeaHistoryReply } from '../../dashboard/ideaHistoryReply.ts';
import type { CardWidgetDeclaration } from '../../families/card/declaration.ts';
import { cardMediaFromIcon, type CardItemRow, type CardModel, type CardProjection } from '../../families/card/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { knownValue, notRecorded } from '../../families/value.ts';
import { formatDateHover } from '../../families/valueFormat.ts';
import type { IdeaHistoryReply } from '../../generated/dashboard';

export type NationalSpiritsOptions = Record<string, never>;

const COLUMNS = { art: { width: 18, height: 18 }, figure: 150, figureAlign: 'right' } as const;
const NO_ICON = 'The spirit has no icon in the current installation';
const NATIONAL_SPIRIT = 'national_spirit';
const UNCLASSIFIED = 'The idea is in no category the content defines';

export const nationalSpiritsProjection: CardProjection<NationalSpiritsOptions> = registerFamilyProjection({
  family: 'card' as const,
  kind: 'national-spirits',
  nextChange(readView) {
    return readView.panel<IdeaHistoryReply>('main')?.nextChange ?? Infinity;
  },
  project(readView): CardModel {
    const hour = readView.hour;
    const reply = readView.panel<IdeaHistoryReply>('main');
    if (!reply) return { hour, rows: [], columns: COLUMNS, rowHeight: 26 };
    const warnings = reply.warnings.length ? { warnings: reply.warnings } : {};
    const nextChangeAt = reply.nextChange ?? Infinity;
    if (reply.unrecorded) {
      return { hour, rows: [], columns: COLUMNS, rowHeight: 26, empty: 'National spirits were not recorded at this date.', emptyKind: 'unavailable', ...warnings, nextChangeAt };
    }
    const spiritCategories = new Set(reply.categories.filter(category => category.kind === NATIONAL_SPIRIT).map(category => category.token));
    const rows: CardItemRow[] = [];
    for (const at of reply.held) {
      const holding = reply.holdings[at]!;
      const look = lookOf(reply, holding);
      const category = look.info.category;
      if (category !== null && !spiritCategories.has(category)) continue;
      const unclassified = category === null ? look.info.unknown ?? UNCLASSIFIED : null;
      const nameMissing = ideaNameMissing(look.artwork, look.info);
      const name = holdingLabel(look, holding);
      const since = holdingSince(holding);
      const icon = look.artwork.icon ?? undefined;
      const shown = nameMissing ? 'Name not recorded' : name;
      rows.push({
        kind: 'item',
        key: holding.token,
        art: cardMediaFromIcon(readView, icon, NO_ICON, nameMissing ? undefined : name),
        title: nameMissing ? notRecorded(nameMissing) : knownValue(name),
        figure: { value: knownValue(`${since.label} ${since.value}`), figure: 'Since', muted: true },
        tooltip: {
          title: shown,
          fullText: shown,
          subtitle: unclassified ? 'Category not recorded' : 'National spirit',
          hour,
          items: [
            { label: since.label, value: formatDateHover(holding.firstHour) },
            ...(unclassified ? [{ label: 'Category', missingReason: unclassified }] : []),
            ...(nameMissing ? [{ label: 'Name', missingReason: nameMissing }] : []),
            ...(icon ? [] : [{ label: 'Art', missingReason: `${NO_ICON}; art missing from the current installation` }]),
          ],
        },
      });
    }
    return {
      hour,
      rows,
      columns: COLUMNS,
      rowHeight: 26,
      ...(rows.length === 0 ? { empty: 'No national spirits at this date.' } : {}),
      ...warnings,
      nextChangeAt,
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
            buildSpanParams: (base, span) => ({ ...base, hour: span.lastHour }),
            provenInterval: ideaHistoryProof,
            validate: validateIdeaHistoryReply,
          },
        }]
      : [],
  }),
  projection: nationalSpiritsProjection,
  projectionOptions: () => ({}),
};
