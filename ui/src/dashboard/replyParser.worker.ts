// Parses reply bytes off the page's thread; the parsed value goes back by structured clone, a chart tile's bins
// decoded here and handed over without a copy.
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
