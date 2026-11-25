import { countryKey } from '../../dashboard/countryKey.ts';
import { flagChoiceArt } from '../../families/art/provider.ts';
import { breakdownAtHourModel, breakdownAtHourSource, validateBreakdownAtHour } from '../../families/breakdown/atHour.ts';
import type { BreakdownWidgetSpec } from '../../families/breakdown/declaration.ts';
import type { BreakdownModel, BreakdownPart, BreakdownProjection } from '../../families/breakdown/model.ts';
import type { BoundedReadView } from '../../families/plan.ts';
import { identityNameOr } from '../../families/table/identityCell.ts';
import { formatRational } from '../../families/valueFormat.ts';
import type { BreakdownPart as WirePart, CasualtyBreakdownReply } from '../../generated/dashboard';

export type CasualtiesBreakdownOptions = Readonly<Record<string, never>>;

const validate = validateBreakdownAtHour<CasualtyBreakdownReply>('casualties breakdown');
const SLOT = 'slot:';

function enemy(readView: BoundedReadView, part: BreakdownPart, wire: WirePart | undefined): BreakdownPart {
  if (!wire || part.neutral || !wire.key.startsWith(SLOT)) return part;
  const shown = Math.floor(readView.hour);
  const atHour = wire.last === null ? shown : Math.min(wire.last, shown);
  const key = countryKey(Number(wire.key.slice(SLOT.length)));
  const identity = readView.who('country', key, atHour);
  return {
    ...part,
    name: identityNameOr(readView, 'country', key, part.name, atHour),
    flag: identity.tag?.status === 'ready' ? identity.tag.value : part.name,
    flagArt: flagChoiceArt(readView, identity.flagChoice),
    ...(identity.colour?.status === 'ready' ? { color: identity.colour.value } : {}),
  };
}

export const casualtiesBreakdownProjection: BreakdownProjection<CasualtiesBreakdownOptions> = {
  family: 'breakdown',
  kind: 'casualties-breakdown',
  project(readView: BoundedReadView): BreakdownModel {
    const reply = readView.panel<CasualtyBreakdownReply>('main');
    const model = breakdownAtHourModel(Math.floor(readView.hour), reply, {
      unit: 'casualties',
      verb: 'Lost',
      since: 'Lost since',
      nothing: 'casualties',
      notRecorded: 'Casualties were not recorded.',
      format: (numerator, denominator) => formatRational(numerator, denominator, { maxDecimals: 0 }),
    });
    if (!reply) return model;
    return {
      ...model,
      parts: model.parts.map((war, at) => {
        const children = reply.breakdown.parts[at]?.children ?? [];
        return war.children ? { ...war, children: war.children.map((child, index) => enemy(readView, child, children[index])) } : war;
      }),
    };
  },
};

export const casualtiesBreakdown: BreakdownWidgetSpec<CasualtiesBreakdownOptions> = {
  kind: 'casualties-breakdown',
  label: 'Casualties breakdown',
  subject: 'casualties',
  emptyCampaign: 'Load a campaign to see its casualties.',
  icon: 'sunburst',
  countries: { mode: 'single', choose: 'Choose a country to see the casualties it took.' },
  projection: casualtiesBreakdownProjection,
  projectionOptions: () => ({}),
  sources: ctx => breakdownAtHourSource(ctx, 'casualty-breakdown', {}, validate),
};
