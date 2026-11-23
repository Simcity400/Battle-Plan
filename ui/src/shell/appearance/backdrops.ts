import { newId } from '../ids.ts';
import { tracked } from '../writes';

export interface BackdropImage { id: string; name: string; image: string; thumbnail: string; width: number; height: number; added: number }

export const MAX_BACKDROPS = 12;
const MAX_INPUT_BYTES = 40 * 1024 * 1024;
const MAX_EDGE = 2560;
const THUMB_EDGE = 360;
const DATABASE = 'BattlePlan', STORE = 'backdrops';

let database: Promise<IDBDatabase> | null = null;
function open(): Promise<IDBDatabase> {
  database ??= new Promise((resolve, reject) => {
    const request = indexedDB.open(DATABASE, 1);
    request.onupgradeneeded = () => { if (!request.result.objectStoreNames.contains(STORE)) request.result.createObjectStore(STORE, { keyPath: 'id' }); };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => { database = null; reject(request.error ?? new Error('Background storage is unavailable.')); };
  });
  return database;
}

function run<T>(mode: IDBTransactionMode, action: (store: IDBObjectStore) => IDBRequest<T>): Promise<T> {
  return mode === 'readwrite' ? tracked(() => transact(mode, action)) : transact(mode, action);
}

async function transact<T>(mode: IDBTransactionMode, action: (store: IDBObjectStore) => IDBRequest<T>): Promise<T> {
  const db = await open();
  return new Promise((resolve, reject) => {
    const transaction = db.transaction(STORE, mode);
    const request = action(transaction.objectStore(STORE));
    transaction.oncomplete = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error('Background storage failed.'));
    transaction.onabort = () => reject(transaction.error ?? new Error('Background storage failed.'));
  });
}

export async function listBackdrops(): Promise<BackdropImage[]> {
  const items = await run<BackdropImage[]>('readonly', store => store.getAll());
  return items.sort((a, b) => a.added - b.added);
}
export const getBackdrop = (id: string) => run<BackdropImage | undefined>('readonly', store => store.get(id));
export const deleteBackdrop = (id: string) => run('readwrite', store => store.delete(id)).then(() => undefined);
export const saveBackdrop = (image: BackdropImage) => run('readwrite', store => store.put(image)).then(() => undefined);

function encode(bitmap: ImageBitmap, edge: number, quality: number): { url: string; width: number; height: number } {
  const scale = Math.min(1, edge / Math.max(bitmap.width, bitmap.height));
  const width = Math.max(1, Math.round(bitmap.width * scale)), height = Math.max(1, Math.round(bitmap.height * scale));
  const canvas = document.createElement('canvas');
  canvas.width = width; canvas.height = height;
  const context = canvas.getContext('2d')!;
  context.imageSmoothingQuality = 'high';
  context.drawImage(bitmap, 0, 0, width, height);
  const webp = canvas.toDataURL('image/webp', quality);
  return { url: webp.startsWith('data:image/webp') ? webp : canvas.toDataURL('image/jpeg', quality), width, height };
}

export const importBackdrop = (file: File): Promise<BackdropImage> => tracked(() => importing(file));

async function importing(file: File): Promise<BackdropImage> {
  if (!file.type.startsWith('image/')) throw new Error('Choose a PNG, JPEG, WebP, GIF or BMP image.');
  if (file.size > MAX_INPUT_BYTES) throw new Error('Choose an image smaller than 40 MB.');
  if ((await listBackdrops()).length >= MAX_BACKDROPS) throw new Error(`You can keep up to ${MAX_BACKDROPS} backgrounds. Remove one first.`);
  let bitmap: ImageBitmap;
  try { bitmap = await createImageBitmap(file); } catch { throw new Error('This image couldn’t be read.'); }
  try {
    const full = encode(bitmap, MAX_EDGE, 0.86);
    const thumb = encode(bitmap, THUMB_EDGE, 0.8);
    const entry: BackdropImage = { id: newId().replaceAll('-', ''), name: file.name.replace(/\.[^.]+$/, '').slice(0, 80) || 'Background',
      image: full.url, thumbnail: thumb.url, width: full.width, height: full.height, added: Date.now() };
    await saveBackdrop(entry);
    return entry;
  } finally { bitmap.close(); }
}
