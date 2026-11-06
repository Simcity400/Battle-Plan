// Parses reply bytes off the page's thread; the parsed value goes back by structured clone.
const decoder = new TextDecoder();
self.onmessage = (event: MessageEvent<{ id: number; bytes: ArrayBuffer }>) => {
  const { id, bytes } = event.data;
  try {
    self.postMessage({ id, value: JSON.parse(decoder.decode(bytes)) as unknown });
  } catch (error) {
    self.postMessage({ id, error: error instanceof Error ? error.message : String(error) });
  }
};
