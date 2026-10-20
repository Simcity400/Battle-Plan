// Turns a picture the user chose into an icon: a small square PNG kept as a data URL in
// the appearance preferences, so it shows without the file and saves with the rest.
import { isIconImage } from './iconStyle.ts';
import type { IconImage } from './iconStyle.ts';

/** Edge of the square PNG in pixels: sharp at the largest place an icon is drawn on a high-density screen. */
export const ICON_IMAGE_SIZE = 64;
/** What the file chooser offers. */
export const ICON_IMAGE_TYPES = 'image/svg+xml,.svg,image/png,image/jpeg,image/webp,image/gif';

/** Where a picture of this size sits in the square: whole, centred, its proportions kept. A picture
 * without its own size (an SVG with only a view box) fills it. */
export function fitIconImage(width: number, height: number, size = ICON_IMAGE_SIZE) {
  if (!(width > 0 && height > 0)) return { x: 0, y: 0, width: size, height: size };
  const scale = size / Math.max(width, height);
  const fitted = { width: width * scale, height: height * scale };
  return { x: (size - fitted.width) / 2, y: (size - fitted.height) / 2, ...fitted };
}

/** The file's bytes as a data URL. The desktop app's content policy admits data: images but not blob: ones. */
function dataUrl(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(blob);
  });
}

/** The file as an icon, or null when the browser cannot decode or draw it. */
export async function rasteriseIconImage(file: File): Promise<IconImage | null> {
  // An SVG only decodes under its own type, which some systems do not give the file.
  const svg = /\.svg$/i.test(file.name) && file.type !== 'image/svg+xml';
  try {
    const image = new Image();
    image.src = await dataUrl(svg ? new Blob([file], { type: 'image/svg+xml' }) : file);
    await image.decode();
    const canvas = document.createElement('canvas');
    canvas.width = canvas.height = ICON_IMAGE_SIZE;
    const context = canvas.getContext('2d');
    if (!context) return null;
    context.imageSmoothingQuality = 'high';
    const box = fitIconImage(image.naturalWidth, image.naturalHeight);
    context.drawImage(image, box.x, box.y, box.width, box.height);
    const data = canvas.toDataURL('image/png');
    return isIconImage(data) ? data : null;
  } catch {
    return null;
  }
}

/** The file drawn as `id`'s icon, or null when a later choice for that icon (another glyph, a reset, another upload)
 * was made while it was being drawn: the newest choice wins. `choices` counts the choices made for each id; choosing
 * the file is itself a choice, so of two uploads in flight only the later one can land, whichever finishes first. */
export async function latestIconImage<Id>(id: Id, file: File, choices: Map<Id, number>,
  rasterise: (file: File) => Promise<IconImage | null> = rasteriseIconImage): Promise<{ image: IconImage | null } | null> {
  const mine = (choices.get(id) ?? 0) + 1;
  choices.set(id, mine);
  const image = await rasterise(file);
  return choices.get(id) === mine ? { image } : null;
}
