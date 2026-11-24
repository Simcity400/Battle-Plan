import { failureText, type Failure } from '../dashboard/queryError.ts';
import { Icon } from './Icons';
import type { IconName } from './Icons';
import '../charts/charts.css';

const GRAPHEMES = new Intl.Segmenter(undefined, { granularity: 'grapheme' });

export function artInitials(name: string): string {
  const words = name.trim().split(/[\s/_-]+/).filter(Boolean);
  if (!words.length) return '';
  const graphemes = (text: string) => [...GRAPHEMES.segment(text)].map(part => part.segment);
  if (words.length === 1) return graphemes(words[0]).slice(0, 2).join('').toUpperCase();
  return words.slice(0, 3).map(word => graphemes(word)[0] ?? '').join('').toUpperCase();
}

export function artFallbackText({ name, failure, reason }: { name?: string | null; failure?: Failure; reason?: string | null }): string | undefined {
  const who = name?.trim();
  const why = failure ? failureText(failure, 'Couldn’t load artwork.') : reason?.trim() || undefined;
  if (who && why) return `${who}: ${why}`;
  if (who) return `${who} — Art not in the current installation.`;
  return why;
}

export default function ArtFallback({ icon, name, className, failure, reason }: {
  icon: IconName; name?: string | null; className?: string; failure?: Failure; reason?: string | null;
}) {
  const label = artFallbackText({ name, failure, reason });
  const tag = name ? artInitials(name) : '';
  return <span className={className ? `art-fallback ${className}` : 'art-fallback'}
    data-has-tag={tag ? '' : undefined}
    data-detail={failure?.message}
    data-tooltip={label}
    aria-hidden={label ? undefined : true}
    role={label ? 'img' : undefined}
    aria-label={label}>
    {tag && <span className="art-fallback-tag" aria-hidden="true">{tag}</span>}
    <Icon name={icon} className="art-fallback-glyph" />
  </span>;
}
