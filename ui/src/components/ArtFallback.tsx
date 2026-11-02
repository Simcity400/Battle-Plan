import { failureText, type Failure } from '../dashboard/queryError.ts';
import { Icon } from './Icons';
import type { IconName } from './Icons';
import '../charts/charts.css';

/** What fills a picture slot when the game has no picture for it: a quiet icon of the
 * subject, so rows and cards keep their shape without a broken-looking box. The name
 * beside it already identifies the item, so the tile is decorative. */
export default function ArtFallback({ icon, className, failure, reason }: { icon: IconName; className?: string; failure?: Failure; reason?: string | null }) {
  return <span className={className ? `art-fallback ${className}` : 'art-fallback'} data-detail={failure?.message} data-tooltip={failure ? failureText(failure, "Couldn’t load artwork.") : reason ?? undefined} aria-hidden={failure || reason ? undefined : true} role={failure || reason ? "img" : undefined} aria-label={failure ? failureText(failure, "Couldn’t load artwork.") : reason ?? undefined}><Icon name={icon} /></span>;
}
