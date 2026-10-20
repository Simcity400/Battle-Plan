import { Icon } from '../../components/Icons';
import type { IconName } from '../../components/Icons';
import { isIconImage } from './iconStyle';
import type { IconStyle } from './iconStyle';
import './iconStyle.css';

/** An icon the user may have changed: its own glyph in a colour, or an uploaded image in the
 * image's own colours. Without one it draws the default glyph. */
export function StyledIcon({ icon, fallback, colour, className }: { icon?: IconStyle['icon']; fallback: IconName; colour?: string; className?: string }) {
  if (isIconImage(icon)) return <img className={`icon-image${className ? ` ${className}` : ''}`} src={icon} alt="" aria-hidden="true" draggable={false} />;
  return <Icon name={icon ?? fallback} className={className} style={colour ? { color: colour } : undefined} />;
}
