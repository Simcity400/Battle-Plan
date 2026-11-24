import { liveKey } from '../../charts/live.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { contentKey, contentSet } from '../../dashboard/options.ts';
import type { TooltipListItem } from '../../families/host/index.ts';
import type { MeterWidgetDeclaration } from '../../families/meter/declaration.ts';
import type { MeterModel, MeterProjection, MeterRow, MeterSection } from '../../families/meter/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { MasteryReward } from '../../generated/dashboard';
import {
  activeTracks,
  choiceName,
  dailyText,
  folderName,
  masteryText,
  rateText,
  rewardName,
  trackKey,
  trackMarks,
  trackName,
  trackRatio,
  trackScale,
  unlockedRewards,
  validateDoctrineMastery,
  type TrackNumbers,
} from './doctrineMasteryData.ts';

export interface DoctrineMasteryProjectionOptions {
  readonly folders: readonly string[];
  readonly countryName: string;
}

export interface DoctrineTrackMeta {
  readonly folderOrdinal: number;
  readonly trackOrdinal: number;
  readonly folderTitle: string;
  readonly title: string;
  readonly subId: string | null;
  readonly rewards: readonly MasteryReward[];
  readonly rewardsUnknown: string | null;
  readonly unlocked: number;
  readonly masteryRaw: string | null;
}

function readTrackNumbers(
  live: ReturnType<Parameters<MeterProjection['project']>[0]['live']>,
  key: string,
): TrackNumbers {
  const read = (column: string): bigint | null => {
    const val = live?.get(liveKey(key, column));
    if (val === undefined) return null;
    if (typeof val === 'bigint') return val;
    return typeof val === 'number' ? BigInt(Math.round(val)) : null;
  };
  const mastery = read('mastery');
  return {
    mastery,
    bank: read('bank'),
    daily: read('daily'),
    effective: read('effective'),
    missing: mastery === null ? (live?.missing(liveKey(key, 'mastery')) || 'Not recorded') : '',
  };
}

export const doctrineMasteryProjection: MeterProjection<DoctrineMasteryProjectionOptions> = registerFamilyProjection({
  family: 'meters',
  kind: 'doctrine-mastery',
  project(readView, options): MeterModel {
    const refused = readView.refused('main');
    if (refused) return { hour: readView.hour, layout: 'columns', refused };
    const raw = readView.panel<unknown>('main');
    if (!raw) return { hour: readView.hour, layout: 'columns', empty: null };
    const reply = validateDoctrineMastery(raw);
    const live = readView.live('main');

    if (!reply.folders.length) {
      return {
        hour: readView.hour,
        layout: 'columns',
        summaryText: '0 tracks active',
        empty: 'No doctrine chosen yet.',
        emptyKind: 'empty',
        warnings: reply.warnings,
      };
    }

    const chosen = options.folders;
    const shownFolders = chosen.length
      ? reply.folders.filter(folder => chosen.includes(folder.id))
      : reply.folders;
    const activeCount = activeTracks(reply.folders);
    const summary = `${activeCount} ${activeCount === 1 ? 'track' : 'tracks'} active`;

    if (!shownFolders.length) {
      return {
        hour: readView.hour,
        layout: 'columns',
        summaryText: summary,
        empty: 'No doctrine folders selected.',
        emptyKind: 'empty',
        warnings: reply.warnings,
      };
    }

    const sections: MeterSection[] = [];
    for (const folder of shownFolders) {
      const fName = folderName(folder);
      const grand = folder.grand ? choiceName(folder.grand, '') : '';
      const sectionTitle = grand ? `${fName} \u00b7 ${grand}` : fName;
      const rows: MeterRow[] = [];

      for (const item of folder.tracks) {
        const key = trackKey(folder, item);
        const numbers = readTrackNumbers(live, key);
        const scale = trackScale(item);
        const ratio = trackRatio(item, numbers.mastery);
        const tName = trackName(item);
        const subTitle = item.sub !== null ? choiceName(item.sub, tName) : 'No subdoctrine chosen';
        const ticks = trackMarks(item, false).map(mark => ({ at: mark.at, reached: mark.reached }));
        const totalSteps = item.rewards.length;

        let subtitle: string;
        if (item.sub === null) {
          subtitle = tName;
        } else if (item.rewardsUnknown !== null) {
          subtitle = `${item.unlocked} ${item.unlocked === 1 ? 'step' : 'steps'} unlocked`;
        } else if (totalSteps === 0) {
          subtitle = tName;
        } else if (item.unlocked >= totalSteps) {
          subtitle = `All ${totalSteps} ${totalSteps === 1 ? 'step' : 'steps'} unlocked`;
        } else {
          const nextReward = item.rewards[item.unlocked]!;
          subtitle = `Step ${item.unlocked + 1} of ${totalSteps} \u00b7 ${rewardName(nextReward)}`;
        }

        const bankLabel = numbers.bank !== null && numbers.bank > 0n
          ? `${masteryText(numbers.bank)} banked`
          : undefined;
        const figure = item.sub !== null && numbers.mastery !== null
          ? (scale !== null ? `${masteryText(numbers.mastery)} / ${masteryText(scale)}` : masteryText(numbers.mastery))
          : undefined;
        const secondary = item.sub !== null
          ? (numbers.mastery !== null ? dailyText(numbers.effective) || undefined : undefined)
          : bankLabel;
        const compactFigure = item.sub !== null
          ? (item.rewardsUnknown !== null ? `${item.unlocked} unlocked` : totalSteps > 0 ? `${item.unlocked}/${totalSteps}` : (numbers.mastery !== null ? masteryText(numbers.mastery) : undefined))
          : '\u2014';

        const tooltipItems: TooltipListItem[] = [
          { label: 'Folder', value: sectionTitle },
          { label: 'Track', value: tName },
          {
            label: 'Mastery',
            ...(numbers.mastery === null
              ? { missingReason: numbers.missing || 'Not recorded' }
              : { value: `${masteryText(numbers.mastery)}${scale !== null ? ` of ${masteryText(scale)}` : ''}` }),
          },
          {
            label: 'Daily gain',
            ...(numbers.effective === null
              ? { missingReason: 'Not recorded' }
              : { value: `${numbers.effective < 0n ? '' : '+'}${rateText(numbers.effective)}` }),
          },
          {
            label: 'Units’ daily gain',
            ...(numbers.daily === null
              ? { missingReason: 'Not recorded' }
              : { value: `${numbers.daily < 0n ? '' : '+'}${rateText(numbers.daily)}` }),
          },
          {
            label: 'Banked mastery',
            ...(numbers.bank === null
              ? { missingReason: 'Not recorded' }
              : { value: masteryText(numbers.bank) }),
          },
          ...(item.rewardsUnknown !== null
            ? [{ label: 'Steps unlocked', value: String(item.unlocked), divider: true }, { label: 'Steps in all', missingReason: item.rewardsUnknown }]
            : [{ label: 'Steps unlocked', value: `${item.unlocked} of ${totalSteps}`, divider: true }]),
          ...unlockedRewards(item).map(r => ({ label: rewardName(r), indent: true })),
          ...(folder.completed.length
            ? [
                { label: 'Completed subdoctrines', divider: true },
                ...folder.completed.map(sub => ({ label: choiceName(sub, ''), indent: true })),
              ]
            : []),
        ];

        const meta: DoctrineTrackMeta = {
          folderOrdinal: folder.ordinal,
          trackOrdinal: item.ordinal,
          folderTitle: sectionTitle,
          title: item.sub !== null ? subTitle : tName,
          subId: item.sub?.id ?? null,
          rewards: item.rewards,
          rewardsUnknown: item.rewardsUnknown,
          unlocked: item.unlocked,
          masteryRaw: numbers.mastery !== null ? numbers.mastery.toString() : null,
        };

        const missingMastery = item.sub !== null && numbers.mastery === null;
        rows.push({
          key,
          name: subTitle,
          subtitle,
          muted: item.sub === null,
          bar: item.sub === null
            ? { kind: 'none', dash: true }
            : { kind: 'threshold', fraction: ratio, ticks, height: 7 },
          figure,
          figureMissing: missingMastery,
          figureMissingLabel: missingMastery ? 'Not recorded' : undefined,
          secondary,
          compactFigure,
          clickable: item.sub !== null && (totalSteps > 0 || item.rewardsUnknown !== null),
          spoken: item.sub === null
            ? `${fName}, ${tName}, No subdoctrine chosen${bankLabel ? `, ${bankLabel}` : ''}`
            : `${fName}, ${subTitle}, ${subtitle}, ${figure ?? 'Not recorded'}${secondary ? `, ${secondary}` : ''}`,
          audit: item.sub === null
            ? `empty|${bankLabel ?? ''}`
            : `${subTitle}|${figure ?? 'not-recorded'}|${secondary ?? ''}|${item.unlocked}/${item.rewardsUnknown !== null ? 'not-known' : totalSteps}`,
          tooltip: {
            title: item.sub !== null ? subTitle : tName,
            hour: readView.hour,
            items: tooltipItems,
          },
          meta,
        });
      }

      if (!folder.tracks.length && !folder.grand) {
        rows.push({
          key: `${folder.id}:none`,
          name: 'No doctrine chosen',
          muted: true,
          bar: { kind: 'none', dash: true },
          compactFigure: '—',
          spoken: `${fName}, No doctrine chosen`,
          audit: 'no-doctrine',
        });
      }
      sections.push({
        key: folder.id,
        title: sectionTitle,
        rows,
      });
    }

    return {
      hour: readView.hour,
      layout: 'columns',
      summaryText: summary,
      sections,
      warnings: reply.warnings,
    };
  },
});

export const doctrineMasteryDeclaration: MeterWidgetDeclaration<DoctrineMasteryProjectionOptions> = {
  family: 'meters',
  kind: 'doctrine-mastery',
  label: 'Doctrine mastery',
  subject: 'doctrines',
  emptyCampaign: 'Load a campaign to see doctrines.',
  icon: 'flask',
  countries: { mode: 'single', choose: 'Choose a country to see its doctrines.' },
  sources: ctx => {
    const params = ctx.countryTag
      ? { branchId: ctx.branch.id, country: countryFromKey(ctx.countryTag) }
      : null;
    return {
      owner: `doctrine-mastery:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${ctx.countryTag ?? ''}`,
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
              product: 'doctrine-mastery',
              followQuery: { panel: 'doctrine-mastery', ...params },
              params,
              validate: validateDoctrineMastery,
            },
          }]
        : [],
    };
  },
  projection: doctrineMasteryProjection,
  projectionOptions: ctx => ({
    folders: contentSet(ctx.options.folders).map(contentKey),
    countryName: ctx.countryName,
  }),
  jumpToDate: true,
};
