import { decodeTileBins } from './tiles/tile.ts';

const decoder = new TextDecoder();
self.onmessage = (event: MessageEvent<{ id: number; bytes: ArrayBuffer }>) => {
  const { id, bytes } = event.data;
  try {
    const value = JSON.parse(decoder.decode(bytes)) as unknown;
    self.postMessage({ id, value }, { transfer: decodeTileBins(value) });
  } catch (error) {
    self.postMessage({ id, error: error instanceof Error ? error.message : String(error) });
  }
};
