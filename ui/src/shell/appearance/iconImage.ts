import { isIconImage } from './iconStyle.ts';
import type { IconImage } from './iconStyle.ts';

export const ICON_IMAGE_SIZE = 64;
export const ICON_IMAGE_TYPES = 'image/svg+xml,.svg,image/png,image/jpeg,image/webp,image/gif';

export function fitIconImage(width: number, height: number, size = ICON_IMAGE_SIZE) {
  if (!(width > 0 && height > 0)) return { x: 0, y: 0, width: size, height: size };
  const scale = size / Math.max(width, height);
  const fitted = { width: width * scale, height: height * scale };
  return { x: (size - fitted.width) / 2, y: (size - fitted.height) / 2, ...fitted };
}

function dataUrl(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(blob);
  });
}

export async function rasteriseIconImage(file: File): Promise<IconImage | null> {
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

export async function latestIconImage<Id>(id: Id, file: File, choices: Map<Id, number>,
  rasterise: (file: File) => Promise<IconImage | null> = rasteriseIconImage): Promise<{ image: IconImage | null } | null> {
  const mine = (choices.get(id) ?? 0) + 1;
  choices.set(id, mine);
  const image = await rasterise(file);
  return choices.get(id) === mine ? { image } : null;
}
