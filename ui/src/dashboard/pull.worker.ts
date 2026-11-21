import { readStream } from './pull.ts';
import { FollowDecoder, type Release } from './follow/decode.ts';
import type { FollowItem } from './follow/message';

self.onmessage = ({ data }: MessageEvent<{ session: string; ports: MessagePort[] }>) => {
  const { ports } = data;
  const decoder = new FollowDecoder();
  for (const port of ports) port.onmessage = ({ data }: MessageEvent<Release>) => decoder.release(data);
  const failure = (reason: string): FollowItem => ({ kind: 'failure', reason });
  void readStream({ session: data.session,
    ready: () => self.postMessage('ready'), fail: reason => { for (const port of ports) port.postMessage([failure(reason)]); self.postMessage({ reason }); },
    answer: async buffers => {
      const items = buffers.map(buffer => {
        try { return decoder.item(buffer); } catch (error) { return failure(String(error)); }
      });
      const transfer = new Set<ArrayBuffer>();
      for (const item of items) if (item.kind === 'followed') {
        for (const { answer } of item.message.answers) if (answer.kind !== 'notRecorded') transfer.add(answer.views.u8.buffer as ArrayBuffer);
      }
      ports.forEach((port, at) => port.postMessage(items, at === ports.length - 1 ? [...transfer] : []));
    },
  });
};
