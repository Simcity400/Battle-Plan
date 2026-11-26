import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { createServer } from 'node:http';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import test from 'node:test';
import { builtServer, devServer, formatFixtureLoadError, loadFixture, type ServerFailure } from './native-page.ts';

function wsServer(onClient: (send: (payload: unknown) => void, onMessage: (handler: (message: Record<string, unknown>) => void) => void) => void) {
  const server = createServer();
  server.on('upgrade', (request, socket) => {
    const key = request.headers['sec-websocket-key'];
    const accept = createHash('sha1').update(`${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest('base64');
    socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
    const send = (payload: unknown) => {
      const body = Buffer.from(JSON.stringify(payload), 'utf8');
      const header = body.length < 126
        ? Buffer.from([0x81, body.length])
        : Buffer.from([0x81, 126, (body.length >> 8) & 0xff, body.length & 0xff]);
      socket.write(Buffer.concat([header, body]));
    };
    let handler: ((message: Record<string, unknown>) => void) | undefined;
    let buffer = Buffer.alloc(0);
    socket.on('data', chunk => {
      buffer = Buffer.concat([buffer, chunk]);
      while (buffer.length >= 2) {
        const opcode = buffer[0]! & 0x0f;
        const masked = (buffer[1]! & 0x80) !== 0;
        let len = buffer[1]! & 0x7f;
        let offset = 2;
        if (len === 126) {
          if (buffer.length < 4) return;
          len = buffer.readUInt16BE(2);
          offset = 4;
        }
        const maskLen = masked ? 4 : 0;
        if (buffer.length < offset + maskLen + len) return;
        const mask = masked ? buffer.subarray(offset, offset + 4) : null;
        const payload = Buffer.from(buffer.subarray(offset + maskLen, offset + maskLen + len));
        buffer = buffer.subarray(offset + maskLen + len);
        if (opcode === 8) { socket.end(); return; }
        if (mask) {
          for (let i = 0; i < payload.length; i++) payload[i] = payload[i]! ^ mask[i % 4]!;
        }
        if (opcode === 1) handler?.(JSON.parse(payload.toString('utf8')));
      }
    });
    onClient(send, next => { handler = next; });
  });
  return new Promise<{ url: string; close(): Promise<void> }>(done => {
    server.listen(0, '127.0.0.1', () => {
      const address = server.address();
      assert.ok(address && typeof address !== 'string');
      done({
        url: `ws://127.0.0.1:${address.port}`,
        close: () => new Promise<void>(closed => { server.close(() => closed()); server.closeAllConnections(); }),
      });
    });
  });
}

test('builtServer verifies fixture readiness, sets Content-Length, and records missing module failures', async t => {
  const dir = await mkdtemp(resolve(tmpdir(), 'native-built-test-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const served = await builtServer('test-built', '/test-built-page', dir);
  t.after(() => served.close());

  await assert.rejects(
    () => served.ready(),
    (error: unknown) => {
      assert.ok(error instanceof Error);
      assert.ok(!(error instanceof assert.AssertionError));
      assert.match(error.message, /fixture could not be loaded \(\/fixture\.js\)/);
      assert.match(error.message, /404 \/fixture\.js: \/fixture\.js is not in the built fixture/);
      return true;
    },
  );

  await writeFile(resolve(dir, 'fixture.js'), 'export const loaded = true;\n', 'utf8');
  await served.ready();

  const response = await fetch(`http://127.0.0.1:${served.port}/fixture.js`);
  assert.equal(response.status, 200);
  assert.equal(response.headers.get('content-type'), 'text/javascript');
  assert.equal(Number(response.headers.get('content-length')), Buffer.byteLength('export const loaded = true;\n'));

  const icon = await fetch(`http://127.0.0.1:${served.port}/favicon.ico`);
  assert.equal(icon.status, 204);
});

test('devServer pre-warms static fixture modules and reports server transform faults as fixture load failures', async t => {
  const profile = await mkdtemp(resolve(tmpdir(), 'native-dev-test-'));
  t.after(() => rm(profile, { recursive: true, force: true }));

  const broken = await devServer('test-broken', '/test-broken-page', '/virtual/broken-fixture.ts', profile, [{
    name: 'broken-fixture',
    resolveId(id) {
      if (id === '/virtual/broken-fixture.ts' || id === '/virtual/broken-dep.ts') return id;
      return null;
    },
    load(id) {
      if (id === '/virtual/broken-fixture.ts') return 'import "/virtual/broken-dep.ts";\nexport const ok = true;\n';
      if (id === '/virtual/broken-dep.ts') throw new Error('synthetic transform boom');
      return null;
    },
  }]);
  t.after(() => broken.close());

  await assert.rejects(
    () => broken.ready(),
    (error: unknown) => {
      assert.ok(error instanceof Error);
      assert.ok(!(error instanceof assert.AssertionError));
      assert.match(error.message, /fixture could not be loaded \(\/virtual\/broken-fixture\.ts\)/);
      assert.match(error.message, /server 500 \/virtual\/broken-dep\.ts: synthetic transform boom/);
      return true;
    },
  );

  const ok = await devServer('test-ok', '/test-ok-page', '/virtual/ok-fixture.ts', profile, [{
    name: 'ok-fixture',
    resolveId(id) {
      if (id === '/virtual/ok-fixture.ts' || id === '/virtual/ok-dep.ts') return id;
      return null;
    },
    load(id) {
      if (id === '/virtual/ok-fixture.ts') return 'import { value } from "/virtual/ok-dep.ts";\nexport const total = value;\n';
      if (id === '/virtual/ok-dep.ts') return 'export const value = 7;\n';
      return null;
    },
  }]);
  t.after(() => ok.close());
  await ok.ready();
});

test('loadFixture reports server and browser fetch failures as fixture load errors, distinct from assertions', async t => {
  const failures: ServerFailure[] = [];
  const cdp = await wsServer((send, onMessage) => {
    onMessage(message => {
      if (message.method === 'Network.enable') {
        send({ id: message.id, result: {} });
        return;
      }
      if (message.method === 'Runtime.evaluate') {
        failures.push({ url: '/chunks/missing.js', status: 500, reason: 'transform failed for missing chunk' });
        send({ method: 'Network.requestWillBeSent', params: { requestId: '1', request: { url: 'http://127.0.0.1:1/chunks/missing.js' } } });
        send({ method: 'Network.responseReceived', params: { requestId: '1', response: { url: 'http://127.0.0.1:1/chunks/missing.js', status: 500, statusText: 'Internal Server Error' } } });
        send({ method: 'Network.loadingFailed', params: { requestId: '1', errorText: 'net::ERR_ABORTED', canceled: true } });
        send({
          id: message.id,
          result: {
            exceptionDetails: {
              text: 'Uncaught (in promise) TypeError: Failed to fetch dynamically imported module: http://127.0.0.1:1/fixture.js',
              exception: { description: 'TypeError: Failed to fetch dynamically imported module: http://127.0.0.1:1/fixture.js' },
            },
          },
        });
      }
    });
  });
  t.after(() => cdp.close());

  await assert.rejects(
    () => loadFixture(cdp.url, { fixture: '/fixture.js', failures }, new AbortController().signal),
    (error: unknown) => {
      assert.ok(error instanceof Error);
      assert.ok(!(error instanceof assert.AssertionError));
      assert.match(error.message, /fixture could not be loaded \(\/fixture\.js\)/);
      assert.match(error.message, /server 500 \/chunks\/missing\.js: transform failed for missing chunk/);
      assert.match(error.message, /browser HTTP 500 Internal Server Error/);
      assert.doesNotMatch(error.message, /net::ERR_ABORTED/);
      return true;
    },
  );

  const formatted = formatFixtureLoadError('/entry.tsx', 'TypeError: Failed to fetch', [], [{ url: 'http://127.0.0.1:1/entry.tsx', reason: 'net::ERR_EMPTY_RESPONSE' }]);
  assert.ok(!(formatted instanceof assert.AssertionError));
  assert.match(formatted.message, /fixture could not be loaded \(\/entry\.tsx\): browser net::ERR_EMPTY_RESPONSE/);
});
