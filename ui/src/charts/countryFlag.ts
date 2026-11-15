import { countryFlagUnavailable, fitCountryFlag } from '../components/countryFlagPresentation.ts';
import type { CountryFlagLook } from '../components/countryFlagPresentation';
import { mix } from '../shell/appearance/color.ts';
import { canvasImageRead } from './images.ts';
import type { ChartTokens } from './tokens';
import type { TooltipContent } from './tooltip';
import { fitTooltip } from './tooltip.ts';

/** Preserve the sampled read; a present URL has its own pending/failed decode. */
export function canvasCountryFlag(flag: CountryFlagLook, wake: () => void): CountryFlagLook {
  if (!flag.flag) return flag;
  const read = canvasImageRead(flag.flag, wake);
  if (read.failed) return { flagPending: false, flagReason: countryFlagUnavailable(undefined, false, true).reason };
  if (read.pending) return { flagPending: true, flagReason: flag.flagReason };
  return flag;
}

/** Missing native flags keep the same neutral filled slot; generic art opts out. */
/** The placeholder's fill of each theme's tokens: mixed once, not for every flag of every frame. */
const placeholders = new WeakMap<object, string>();

export function drawCountryFlag(context: CanvasRenderingContext2D, flag: CountryFlagLook | undefined,
  x: number, y: number, width: number, height: number, tokens: Pick<ChartTokens, 'surface' | 'muted' | 'border' | 'lineWidth'>,
  wake: () => void): void {
  if (!flag) return;
  const image = flag.flag ? canvasImageRead(flag.flag, wake).image : null;
  if (image) {
    const fitted = fitCountryFlag(image.naturalWidth, image.naturalHeight, width, height);
    context.drawImage(image, x + fitted.x, y + fitted.y, fitted.width, fitted.height);
    return;
  }
  context.save();
  let fill = placeholders.get(tokens);
  if (fill === undefined) placeholders.set(tokens, fill = mix(tokens.surface, tokens.muted, 0.25));
  context.fillStyle = fill;
  context.fillRect(x, y, width, height);
  context.strokeStyle = tokens.border; context.lineWidth = tokens.lineWidth.grid;
  context.strokeRect(x + context.lineWidth / 2, y + context.lineWidth / 2, Math.max(0, width - context.lineWidth), Math.max(0, height - context.lineWidth));
  context.restore();
}

/** Decode displayed/fitted country rows only; full spoken metadata stays decode-free.
 * A missing flag is decorative, so it adds nothing to what is read out. */
export function countryFlagContent(content: TooltipContent | null, flag: CountryFlagLook | undefined, wake: () => void): TooltipContent | null {
  if (!content) return null;
  if (!flag && !content.countryFlag && !content.rows.some(row => row.countryFlag)) return content;
  const header = flag ?? content.countryFlag;
  const countryFlag = header ? canvasCountryFlag(header, wake) : undefined;
  const rows = content.rows.map(row => row.countryFlag ? { ...row, countryFlag: canvasCountryFlag(row.countryFlag, wake) } : row);
  return { ...content, countryFlag, rows };
}

/** Dated tooltip metadata wins over a node's look. Scrolling rows are decoded
 * only when the shared tooltip layer mounts them. */
export function countryTooltipContent(raw: TooltipContent | null, flag: CountryFlagLook | undefined,
  wake: () => void, height: number, keep = 0): TooltipContent | null {
  if (!raw) return null;
  const content = !raw.countryFlag && flag ? { ...raw, countryFlag: flag } : raw;
  if (content.scrollable) {
    const header = countryFlagContent({ ...content, rows: [] }, undefined, wake)!;
    return { ...header, rows: content.rows };
  }
  const decoded = countryFlagContent(fitTooltip([content], height, { keep }), undefined, wake)!;
  return fitTooltip([decoded], height, { keep });
}
