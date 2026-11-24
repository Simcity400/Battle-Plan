import type { MeterWidgetDeclaration } from '../../families/meter/declaration.ts';
import type {
  MeterChip,
  MeterChipLine,
  MeterFlag,
  MeterModel,
  MeterProjection,
  MeterRow,
  MeterSection,
} from '../../families/meter/model.ts';
import { registerMeterProjection } from '../../families/meter/registry.ts';
import type { CipherPair } from '../../generated/dashboard';
import { pairKey, percentOf, progressLabel, workingStatus } from './decryptionData.ts';
import {
  decryptionFilterOptions,
  decryptionNextChange,
  decryptionSources,
  brokenText,
  pairTooltipItems,
  readCipherNumbers,
  readDecryption,
  type DecryptionFilterOptions,
  type ResolvedCountry,
} from './decryptionRead.ts';

const flagOf = (country: ResolvedCountry): MeterFlag => ({ tag: country.tag, result: country.flag });

export const decryptionProjection: MeterProjection<DecryptionFilterOptions> = registerMeterProjection({
  family: 'meters',
  kind: 'decryption',
  nextChange(readView, options): number {
    return decryptionNextChange(options.filter, readView.hour);
  },
  project(readView, options): MeterModel {
    const layout = 'ciphers';
    const frame = readDecryption(readView, options);
    if (frame.kind === 'refused') return { hour: readView.hour, layout, refused: frame.reason };
    if (frame.kind === 'pending') return { hour: readView.hour, layout, empty: null };
    if (frame.kind === 'empty') {
      return {
        hour: readView.hour,
        layout,
        empty: frame.empty,
        ...(frame.emptyDetail ? { emptyDetail: frame.emptyDetail } : {}),
        emptyKind: frame.emptyKind,
        ...(frame.summaryText ? { summaryText: frame.summaryText } : {}),
        warnings: frame.warnings,
      };
    }
    const country = frame.country;
    const sections: MeterSection[] = [];
    const progressRows = (list: readonly CipherPair[]): MeterRow[] => list.map(pair => {
        const owner = country(pair.owner);
        const target = country(pair.target);
        const numbers = readCipherNumbers(frame.live, pair);
        const percent = percentOf(pair, numbers);
        const key = `active:${pairKey(pair)}`;
        const title = `${owner.name} \u2192 ${target.name}`;
        return {
          key,
          name: title,
          pair: {
            ownerFlag: flagOf(owner),
            ownerName: owner.name,
            targetFlag: flagOf(target),
            targetName: target.name,
          },
          bar: {
            kind: 'track',
            fraction: percent === null ? null : percent / 100,
            color: owner.colour,
            height: 8,
          },
          figure: percent === null ? undefined : `${percent}%`,
          figureMissing: percent === null,
          spoken: `${owner.name} decrypting ${target.name}, ${progressLabel(percent)}, ${workingStatus(pair)}`,
          audit: `${owner.tag}>${target.tag}|${percent === null ? 'not-recorded' : `${percent}%`}`,
          tooltip: {
            title,
            hour: readView.hour,
            items: pairTooltipItems(pair, frame.reply, numbers, owner, target, percent),
          },
        };
      });
    const progressSections: readonly [string, string, readonly CipherPair[]][] = [
      ['decrypting', 'Decrypting', frame.byStatus.decrypting],
      ['paused', 'Paused', frame.byStatus.paused],
      ['break-unknown', 'Not known if broken', frame.byStatus.breakUnknown],
    ];
    for (const [key, title, list] of progressSections) {
      if (list.length) sections.push({ key, title, count: String(list.length), rows: progressRows(list) });
    }

    if (frame.byStatus.broken.length) {
      const grouped = new Map<number, { owner: ResolvedCountry; items: CipherPair[] }>();
      for (const pair of frame.byStatus.broken) {
        const group = grouped.get(pair.owner.id) ?? { owner: country(pair.owner), items: [] };
        group.items.push(pair);
        grouped.set(pair.owner.id, group);
      }
      const chipLines: MeterChipLine[] = [...grouped.values()].map(group => {
        const chips: MeterChip[] = group.items.map(pair => {
          const target = country(pair.target, pair.brokenAt?.hour);
          const numbers = readCipherNumbers(frame.live, pair);
          const percent = percentOf(pair, numbers);
          const dateText = brokenText(pair.brokenAt);
          const title = `${group.owner.name} \u2192 ${target.name}`;
          return {
            key: `broken:${pairKey(pair)}`,
            targetFlag: flagOf(target),
            targetName: target.name,
            dateText,
            spoken: `${group.owner.name} broke ${target.name} cipher${!pair.brokenAt ? ', date not recorded' : pair.brokenAt.upperBound ? ` ${dateText}` : ` on ${dateText}`}`,
            audit: `${group.owner.tag}>${target.tag}|broken|${dateText ?? 'not-recorded'}`,
            tooltip: {
              title,
              hour: readView.hour,
              items: pairTooltipItems(pair, frame.reply, numbers, group.owner, target, percent),
            },
          };
        });
        return {
          key: `owner:${group.owner.key}`,
          ownerFlag: flagOf(group.owner),
          ownerName: group.owner.name,
          chips,
        };
      });
      sections.push({
        key: 'broken',
        title: 'Cipher broken',
        count: String(frame.byStatus.broken.length),
        chipLines,
      });
    }

    return {
      hour: readView.hour,
      layout: 'ciphers',
      summaryText: frame.summaryText,
      sections,
      notes: frame.notes,
      warnings: frame.reply.warnings,
    };
  },
}, 'ciphers');

export const decryptionDeclaration: MeterWidgetDeclaration<DecryptionFilterOptions> = {
  family: 'meters',
  kind: 'decryption',
  label: 'Decryption',
  subject: 'decryption',
  emptyCampaign: 'Load a campaign to see its decryption races.',
  icon: 'lock',
  countries: { mode: 'filter', labels: true, colours: true },
  sources: decryptionSources,
  projection: decryptionProjection,
  projectionOptions: decryptionFilterOptions,
  jumpToDate: true,
  keyByBranch: true,
};
