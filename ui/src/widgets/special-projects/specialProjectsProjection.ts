import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { CardWidgetDeclaration } from '../../families/card/declaration.ts';
import { cardMediaFromIcon, type CardItemRow, type CardModel, type CardProjection, type CardRow } from '../../families/card/model.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { knownValue, notRecorded, type Value } from '../../families/value.ts';
import { formatDateHover, formatRecordedPercent } from '../../families/valueFormat.ts';
import type { ProjectPresentation, ProjectRecord, SpecialProjectsReply } from '../../generated/dashboard';
import { validateSpecialProjectsReply } from './specialProjectsData.ts';

const COLUMNS = { date: true, art: { width: 46, height: 28 }, meter: 200, meterText: 44, figure: 150, figureAlign: 'right' } as const;
const NO_ICON = 'The project has no icon in the current installation';

function scientist(reply: SpecialProjectsReply, info: ProjectPresentation): CardItemRow['figure'] {
  const person = reply.people[info.scientist];
  if (person?.name) return { value: knownValue(person.name), figure: 'Scientist' };
  if (person?.assigned === false) return { value: knownValue('No scientist'), figure: 'Scientist', muted: true };
  return { value: notRecorded(person?.missing ?? 'The scientist was not recorded'), figure: 'Scientist' };
}

function base(readView: BoundedReadView, reply: SpecialProjectsReply, info: ProjectPresentation, extra?: string): Pick<CardItemRow, 'art' | 'title' | 'subtitle' | 'figure'> {
  const subtitle = [info.specializationName, extra].filter(Boolean).join(' · ');
  return {
    art: cardMediaFromIcon(readView, info.icon, info.iconMissing ?? NO_ICON, info.name),
    title: knownValue(info.name),
    ...(subtitle ? { subtitle } : {}),
    figure: scientist(reply, info),
  };
}

function runAt(project: ProjectRecord, hour: number) {
  return project.statuses.find(run => run.firstHour <= hour && hour <= run.lastHour) ?? null;
}

export const specialProjectsProjection: CardProjection<Record<string, never>> = registerFamilyProjection({
  family: 'card' as const,
  kind: 'special-projects',
  project(readView): CardModel {
    const hour = readView.hour;
    const raw = readView.panel<unknown>('main');
    if (raw === undefined) return { hour, rows: [], columns: COLUMNS, rowHeight: 36 };
    const reply = validateSpecialProjectsReply(raw);
    const warnings = reply.warnings.length ? { warnings: reply.warnings } : {};
    if (reply.unavailable) return { hour, rows: [], columns: COLUMNS, rowHeight: 36, empty: reply.unavailable, emptyKind: 'unavailable', ...warnings };
    if (!reply.available.some(span => span.firstHour <= hour && hour <= span.lastHour)) {
      return { hour, rows: [], columns: COLUMNS, rowHeight: 36, empty: 'Special projects were not recorded at this date.', emptyKind: 'unavailable', ...warnings };
    }
    const rows: CardRow[] = [];
    const progressRows = (indexes: readonly number[], section: string, label: string, extraOf: (state: string) => string | undefined) => {
      if (indexes.length) rows.push({ kind: 'section', key: section, label, count: indexes.length });
      for (const index of indexes) {
        const project = reply.projects[index]!;
        const run = runAt(project, hour);
        if (!run) continue;
        const info = reply.presentations[run.presentation]!;
        const fraction: Value<number> = run.progress <= 0 ? { kind: 'zero', value: 0 } : knownValue(run.progress / 100);
        rows.push({
          kind: 'item',
          key: `running:${project.id}`,
          subject: project.id,
          ...base(readView, reply, info, extraOf(run.state)),
          meter: { fraction, text: formatRecordedPercent(run.progress) },
        });
      }
    };
    progressRows(reply.shown.running, '__running', 'In progress', state => (state === 'paused' ? 'Paused' : undefined));
    progressRows(reply.shown.unknown, '__unknown', 'State not recorded', () => undefined);
    const completed = reply.shown.completed;
    const dated = completed.length - reply.shown.undated;
    if (completed.length) rows.push({ kind: 'section', key: '__completed', label: 'Completed', count: completed.length });
    completed.forEach((item, at) => {
      if (at === dated && reply.shown.undated > 0) rows.push({ kind: 'section', key: '__undated', label: 'Date not recorded', count: reply.shown.undated });
      const project = reply.projects[item.project]!;
      const run = runAt(project, hour);
      if (!run) return;
      const info = reply.presentations[run.presentation]!;
      const reason = project.firstState === 'completed' ? 'Already complete when the project was first recorded' : 'The completion date was not recorded';
      rows.push({
        kind: 'item',
        key: `done:${project.id}`,
        subject: project.id,
        date: item.date !== null ? { hour: item.date } : { hour: null, reason },
        ...base(readView, reply, info),
        tooltip: {
          title: info.name,
          fullText: info.name,
          hour,
          items: [
            item.date !== null ? { label: 'Completed', value: formatDateHover(item.date) } : { label: 'Completed', missingReason: reason },
            ...(info.icon ? [] : [{ label: 'Art', missingReason: `${info.iconMissing ?? NO_ICON}; art missing from the current installation` }]),
          ],
        },
      });
    });
    return {
      hour,
      rows,
      columns: COLUMNS,
      rowHeight: 36,
      ...(rows.length === 0 ? { empty: 'No special projects at this date.' } : {}),
      ...warnings,
    };
  },
}, 'live') as CardProjection<Record<string, never>>;

export function plannedCountry(owner: string): string | null {
  return owner.startsWith('special-projects:') ? owner.slice(owner.lastIndexOf(':') + 1) || null : null;
}

export const specialProjectsDeclaration: CardWidgetDeclaration<Record<string, never>> = {
  family: 'card',
  kind: 'special-projects',
  label: 'Special projects',
  subject: 'special projects',
  emptyCampaign: 'Load a campaign to see special projects.',
  icon: 'flask',
  countries: { mode: 'single', choose: 'Choose a country to see its special projects.' },
  keyByBranch: true,
  jumpToDate: true,
  sources: ctx => ({
    owner: `special-projects:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${ctx.countryTag ?? ''}`,
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
            product: 'special-projects',
            followQuery: { panel: 'special-projects', branchId: ctx.branch.id, country: countryFromKey(ctx.countryTag), current: true },
            params: { branchId: ctx.branch.id, country: countryFromKey(ctx.countryTag), current: true },
            validate: validateSpecialProjectsReply,
          },
        }]
      : [],
  }),
  projection: specialProjectsProjection,
  projectionOptions: () => ({}),
};
