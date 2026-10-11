import { Icon } from './Icons';
import type { IconName } from './Icons';
import '../charts/charts.css';

/** What fills a picture slot when the game has no picture for it: a quiet icon of the
 * subject, so rows and cards keep their shape without a broken-looking box. The name
 * beside it already identifies the item, so the tile is decorative. */
export default function ArtFallback({ icon, className }: { icon: IconName; className?: string }) {
  return <span className={className ? `art-fallback ${className}` : 'art-fallback'} aria-hidden="true"><Icon name={icon} /></span>;
}
