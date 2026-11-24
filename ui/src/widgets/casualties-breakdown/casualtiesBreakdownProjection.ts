import { flagChoiceArt } from '../../families/art/provider.ts';
import { breakdownAtHourModel, breakdownAtHourSource, validateBreakdownAtHour } from '../../families/breakdown/atHour.ts';
import type { BreakdownWidgetSpec } from '../../families/breakdown/declaration.ts';
import type { BreakdownModel, BreakdownPart, BreakdownProjection } from '../../families/breakdown/model.ts';
import type { BoundedReadView } from '../../families/plan.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { BreakdownPart as WirePart, CasualtyBreakdownReply } from '../../generated/dashboard';

export type CasualtiesBreakdownOptions = Readonly<Record<string, never>>;

const validate = validateBreakdownAtHour<CasualtyBreakdownReply>('casualties breakdown');

function enemy(readView: BoundedReadView, part: BreakdownPart, wire: WirePart | undefined): BreakdownPart {
  if (!wire || wire.key.startsWith('~')) return part;
  const shown = Math.floor(readView.hour);
  const identity = readView.who('country', wire.key, wire.last === null ? shown : Math.min(wire.last, shown));
  return {
    ...part,
    name: identity.name.status === 'ready' ? identity.name.value : part.name,
    flag: wire.key,
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
      verb: 'Taken',
      since: 'Taken since',
      nothing: 'casualties',
      notRecorded: 'Casualties were not recorded.',
      format: raw => formatScalar(raw, 'integer'),
    });
    if (!reply) return model;
    return {
      ...model,
      parts: model.parts.map((war, at) => {
        const wars = reply.breakdown.parts[at]?.children ?? [];
        return war.children ? { ...war, children: war.children.map((child, index) => enemy(readView, child, wars[index])) } : war;
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
