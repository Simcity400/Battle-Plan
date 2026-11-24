import { liveKey } from '../../charts/live.ts';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { missingArt, NO_ART } from '../../families/art/types.ts';
import type { MeterWidgetDeclaration } from '../../families/meter/declaration.ts';
import type { MeterModel, MeterProjection, MeterRow } from '../../families/meter/model.ts';
import { registerMeterProjection } from '../../families/meter/registry.ts';
import { formatDateHover, formatDateOnly } from '../../families/valueFormat.ts';
import type { ResearchDefinitionText, ResearchSlotsReply } from '../../generated/dashboard';
import { researchArtwork, researchReadName, validateResearchReply, validateResearchSlots } from '../research/researchData.ts';
import { slotNumbers } from '../research/researchSlots.ts';
import type { ResearchData } from '../research/types';
import { activeSlots, openedResearch, type OpenedResearch } from './activeResearchData.ts';

export interface ActiveResearchProjectionOptions {
  readonly history: ResearchData | undefined;
  readonly countryName: string;
}

export interface ActiveResearchRowMeta {
  readonly opened: OpenedResearch;
  readonly name: string;
  readonly slot: number;
  readonly token: string;
  readonly databaseIndex: number;
  readonly definition: ResearchDefinitionText | null;
}

const nextDayAfter = (hour: number) => (Math.floor(hour / 24) + 1) * 24;

function folderNameFor(
  data: ResearchData | undefined,
  token: string,
  databaseIndex: number,
): string | undefined {
  if (!data) return undefined;
  let folderKey: string | null | undefined;
  for (const item of data.technologyFolders) {
    if (item.token !== token) continue;
    if (item.databaseIndex === databaseIndex) {
      folderKey = item.folder;
      break;
    }
    if (item.databaseIndex == null && folderKey === undefined) {
      folderKey = item.folder;
    }
  }
  if (!folderKey) return undefined;
  const folder = data.folders.find(item => item.key === folderKey);
  return folder ? contentTitle(folder.name, folder.key, '') || undefined : undefined;
}

const EMPTY_ARTWORK = new Map<string, never>();

function techNameOf(
  data: ResearchData | undefined,
  read: ResearchSlotsReply['slots'][number],
  token: string,
): string {
  const artwork = researchArtwork(data, EMPTY_ARTWORK);
  const raw = researchReadName(artwork, read);
  return contentTitle(raw, token, 'Technology name unavailable');
}

export const activeResearchProjection: MeterProjection<ActiveResearchProjectionOptions> = registerMeterProjection({
  family: 'meters',
  kind: 'active-research',
  nextChange(readView): number {
    const raw = readView.panel<unknown>('main');
    if (!raw) return Infinity;
    let reply: ResearchSlotsReply;
    try {
      reply = validateResearchSlots(raw);
    } catch {
      return Infinity;
    }
    let next = Infinity;
    for (const slot of reply.slots) {
      if (slot.state.details?.remainingDays != null) {
        const boundary = slot.state.firstHour + nextDayAfter(readView.hour - slot.state.firstHour);
        if (boundary > readView.hour) next = Math.min(next, boundary);
      }
    }
    return next;
  },
  project(readView, options): MeterModel {
    const refused = readView.refused('main');
    if (refused) return { hour: readView.hour, layout: 'rows', refused };
    const raw = readView.panel<unknown>('main');
    if (!raw || !options.history) {
      return { hour: readView.hour, layout: 'rows', empty: null };
    }
    const reply = validateResearchSlots(raw);
    const live = readView.live('main');
    const now = { ...reply, hour: readView.hour };
    const items = activeSlots(options.history.history, now);
    const unreadable = !!reply.missing && items.every(item => item.kind === 'unavailable');

    if (unreadable) {
      return {
        hour: readView.hour,
        layout: 'rows',
        empty: reply.missing,
        emptyKind: 'unavailable',
        warnings: [...options.history.warnings, ...reply.warnings],
      };
    }
    if (!items.length) {
      return {
        hour: readView.hour,
        layout: 'rows',
        empty: `No research slots recorded at ${formatDateOnly(readView.hour)}.`,
        emptyKind: 'empty',
        warnings: [...options.history.warnings, ...reply.warnings],
      };
    }

    let researchingCount = 0;
    let oldestReadHour: number = readView.hour;
    let nextBoundary = Infinity;
    const rows: MeterRow[] = [];

    for (const item of items) {
      const key = `slot:${item.slot.index}`;
      if (item.kind === 'unavailable') {
        rows.push({
          key,
          name: 'Not recorded',
          muted: true,
          art: { result: missingArt(item.reason), resolvedName: 'NR', radius: 4 },
          bar: { kind: 'none' },
          figureMissing: true,
          figureMissingLabel: 'Not recorded',
          spoken: `Slot ${item.slot.index + 1}, not recorded: ${item.reason}`,
          audit: `not-recorded:${item.reason}`,
          tooltip: {
            title: `Research slot ${item.slot.index + 1}`,
            hour: readView.hour,
            missingReason: item.reason,
          },
        });
        continue;
      }

      const liveVal = live?.get(liveKey(String(item.slot.index), 'progress'));
      const progressNum = typeof liveVal === 'number' ? liveVal : typeof liveVal === 'bigint' ? Number(liveVal) : undefined;
      const numbers = slotNumbers(item.read, readView.hour, progressNum);

      if (item.read.state.details?.remainingDays != null) {
        const boundary = item.read.state.firstHour + nextDayAfter(readView.hour - item.read.state.firstHour);
        if (boundary > readView.hour) nextBoundary = Math.min(nextBoundary, boundary);
      }
      if (item.read.state.firstHour < oldestReadHour) {
        oldestReadHour = item.read.state.firstHour;
      }

      if (item.kind === 'idle') {
        rows.push({
          key,
          name: 'Idle',
          muted: true,
          art: { result: NO_ART, idle: true, radius: 4 },
          bar: { kind: 'none' },
          secondary: numbers.saved || undefined,
          spoken: numbers.saved ? `Slot ${item.slot.index + 1}, Idle, ${numbers.saved}` : `Slot ${item.slot.index + 1}, Idle`,
          audit: numbers.saved ? `idle:${numbers.saved}` : 'idle',
          tooltip: {
            title: `Research slot ${item.slot.index + 1}`,
            hour: readView.hour,
            items: [
              { label: 'Status', value: 'Idle' },
              ...(numbers.saved ? [{ label: 'Saved progress', value: numbers.saved }] : []),
            ],
          },
        });
        continue;
      }

      researchingCount++;
      const name = techNameOf(options.history, item.read, item.token);
      const folder = folderNameFor(options.history, item.token, item.databaseIndex);
      const artResult = readView.art({ kind: 'sprite', key: `technology:${item.token}`, frame: 0 });
      const pctText = numbers.fraction === null ? undefined : numbers.percent;
      const daysText = numbers.left === null ? undefined : numbers.days;
      const opened = openedResearch(item);
      const meta: ActiveResearchRowMeta = {
        opened,
        name,
        slot: item.slot.index,
        token: item.token,
        databaseIndex: item.databaseIndex,
        definition: item.read.definition,
      };
      const artMissingNote = artResult.status === 'confirmedMissing'
        ? 'Art missing from the current installation.'
        : undefined;
      const estimateNote = numbers.left !== null
        ? 'Days left is the game’s estimate at this hour’s research speed.'
        : undefined;
      const note = [estimateNote, artMissingNote].filter(Boolean).join(' ') || undefined;

      rows.push({
        key,
        name,
        subtitle: folder,
        art: { result: artResult, resolvedName: name, radius: 4 },
        bar: { kind: 'track', fraction: numbers.fraction, height: 6 },
        figure: pctText,
        figureMissing: numbers.fraction === null,
        figureMissingLabel: numbers.fraction === null ? 'Not recorded' : undefined,
        secondary: daysText,
        clickable: true,
        spoken: `${name}${folder ? `, ${folder}` : ''}, ${numbers.percent}, ${numbers.days}`,
        audit: `${name}|${numbers.percent}|${numbers.days}`,
        tooltip: {
          title: name,
          hour: readView.hour,
          items: [
            ...(folder ? [{ label: 'Category', value: folder }] : []),
            {
              label: 'Progress',
              ...(numbers.fraction === null
                ? { missingReason: 'Research progress not recorded' }
                : { value: `${numbers.percent} on ${formatDateHover(readView.hour)}` }),
            },
            {
              label: 'Days left',
              ...(numbers.left === null
                ? { missingReason: 'Days left estimate not recorded' }
                : { value: String(numbers.left) }),
            },
          ],
          ...(note ? { note } : {}),
        },
        meta,
      });
    }

    const staleCaption = readView.hour - oldestReadHour >= 24
      ? `As of ${formatDateOnly(oldestReadHour)}`
      : undefined;

    return {
      hour: readView.hour,
      layout: 'rows',
      summaryText: null,
      sections: [{
        key: 'researching',
        title: 'Researching',
        count: `${researchingCount} of ${items.length} ${items.length === 1 ? 'slot' : 'slots'}`,
        rightCaption: staleCaption,
        rows,
      }],
      warnings: [...options.history.warnings, ...reply.warnings],
      ...(Number.isFinite(nextBoundary) ? { nextChangeAt: nextBoundary } : {}),
    };
  },
});

export const activeResearchDeclaration: MeterWidgetDeclaration<ActiveResearchProjectionOptions, ResearchData> = {
  family: 'meters',
  kind: 'active-research',
  label: 'Active research',
  subject: 'research',
  emptyCampaign: 'Load a campaign and choose a country to see its research.',
  icon: 'flask',
  countries: { mode: 'single', choose: 'Choose a country to see its research.' },
  aux: {
    product: 'research',
    params: (track, _options, country) => (country ? { branchId: track.branch.id, country: countryFromKey(country) } : null),
    validate: validateResearchReply,
  },
  sources: ctx => {
    const params = ctx.countryTag
      ? { branchId: ctx.branch.id, country: countryFromKey(ctx.countryTag) }
      : null;
    return {
      owner: `active-research:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${ctx.countryTag ?? ''}`,
      entries: params
        ? [{
            name: 'main',
            source: ctx.track.source,
            branch: ctx.branch.id,
            offset: ctx.track.axisOffset,
            read: {
              shape: 'point',
              channel: 'panel',
              coverage: { mode: 'keyed', keys: [ctx.countryTag!] },
              product: 'research-slots',
              followQuery: { panel: 'research-slots', ...params },
              params,
              validate: validateResearchSlots,
            },
          }]
        : [],
    };
  },
  projection: activeResearchProjection,
  projectionOptions: ctx => ({
    history: ctx.aux,
    countryName: ctx.countryName,
  }),
  jumpToDate: true,
};
