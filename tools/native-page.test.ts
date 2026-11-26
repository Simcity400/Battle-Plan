import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { createServer } from 'node:http';
import { connect } from 'node:net';
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

test('builtServer verifies fixture readiness, sets Content-Length, records missing modules, and closes malformed sockets', async () => {
  const dir = await mkdtemp(resolve(tmpdir(), 'native-built-test-'));
  const served = await builtServer('test-built', '/test-built-page', dir);
  try {
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

    const malformed = await new Promise<string>((done, reject) => {
      const socket = connect(served.port, '127.0.0.1', () => { socket.write('BAD HTTP\r\n\r\n'); });
      let received = '';
      socket.on('data', chunk => { received += String(chunk); });
      socket.once('close', () => done(received));
      socket.once('error', reject);
    });
    assert.match(malformed, /^HTTP\/1\.1 400 Bad Request/);
    assert.ok(served.failures.some(entry => entry.url === '<socket>' && entry.reason.startsWith('HTTP client error:')));
  } finally {
    await served.close();
    await rm(dir, { recursive: true, force: true });
  }
});

test('devServer pre-warms virtual HTTP modules, aborts stalled transforms, and captures plugin errors and binary/callback bodies', async () => {
  const brokenProfile = await mkdtemp(resolve(tmpdir(), 'native-dev-broken-'));
  const broken = await devServer('test-broken', '/test-broken-page', '/virtual/broken-fixture.ts', brokenProfile, [{
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
  try {
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
  } finally {
    await broken.close();
    await rm(brokenProfile, { recursive: true, force: true });
  }

  const virtualProfile = await mkdtemp(resolve(tmpdir(), 'native-dev-virtual-'));
  const virtualId = '\0virtual:fixture-library';
  const ok = await devServer('test-ok', '/test-ok-page', '/virtual/ok-fixture.ts', virtualProfile, [{
    name: 'ok-fixture',
    configureServer(server) {
      server.middlewares.use('/plugin-503-bytes', (_request, response) => {
        response.statusCode = 503;
        response.end(new Uint8Array(Buffer.from('transform unavailable', 'utf8')));
      });
      server.middlewares.use('/plugin-503-cb', (_request, response) => {
        response.statusCode = 503;
        response.statusMessage = 'Service Unavailable';
        response.end(() => {});
      });
      server.middlewares.use('/truncated-200', (_request, response) => {
        response.writeHead(200, { 'Content-Type': 'text/javascript', 'Content-Length': '100' });
        response.write('export const partial = 1;');
        response.socket?.destroy();
      });
    },
    resolveId(id) {
      if (id === '/virtual/ok-fixture.ts') return id;
      if (id === 'virtual:fixture-library') return virtualId;
      return null;
    },
    load(id) {
      if (id === '/virtual/ok-fixture.ts') return 'import { value } from "virtual:fixture-library";\nexport const total = value;\n';
      if (id === virtualId) return 'export const value = 7;\n';
      return null;
    },
  }]);
  try {
    await ok.ready();
    assert.equal((await fetch(`http://127.0.0.1:${ok.port}/plugin-503-bytes`)).status, 503);
    assert.equal((await fetch(`http://127.0.0.1:${ok.port}/plugin-503-cb`)).status, 503);
    assert.ok(ok.failures.some(entry => entry.url === '/plugin-503-bytes' && entry.status === 503 && entry.reason === 'transform unavailable'));
    assert.ok(ok.failures.some(entry => entry.url === '/plugin-503-cb' && entry.status === 503 && entry.reason === 'Service Unavailable'));
  } finally {
    await ok.close();
    await rm(virtualProfile, { recursive: true, force: true });
  }

  const truncatedProfile = await mkdtemp(resolve(tmpdir(), 'native-dev-trunc-'));
  const truncated = await devServer('test-trunc', '/test-trunc-page', '/truncated-entry.js', truncatedProfile, [{
    name: 'truncated-fixture',
    configureServer(server) {
      server.middlewares.use('/truncated-entry.js', (_request, response) => {
        response.writeHead(200, { 'Content-Type': 'text/javascript', 'Content-Length': '128' });
        response.write('export const cut = 1;');
        response.socket?.destroy();
      });
    },
  }]);
  try {
    await assert.rejects(
      () => truncated.ready(),
      (error: unknown) => {
        assert.ok(error instanceof Error);
        assert.ok(!(error instanceof assert.AssertionError));
        assert.match(error.message, /fixture could not be loaded \(\/truncated-entry\.js\)/);
        assert.match(error.message, /UND_ERR_SOCKET/);
        return true;
      },
    );
  } finally {
    await truncated.close();
    await rm(truncatedProfile, { recursive: true, force: true });
  }

  const stalledProfile = await mkdtemp(resolve(tmpdir(), 'native-dev-stalled-'));
  let releaseTransform!: () => void;
  let enteredTransform!: () => void;
  const transformEntered = new Promise<void>(done => { enteredTransform = done; });
  const transformHold = new Promise<void>(done => { releaseTransform = done; });
  const stalled = await devServer('test-stalled', '/test-stalled-page', '/virtual/stalled-fixture.ts', stalledProfile, [{
    name: 'stalled-fixture',
    resolveId(id) {
      return id === '/virtual/stalled-fixture.ts' ? id : null;
    },
    async load(id) {
      if (id !== '/virtual/stalled-fixture.ts') return null;
      enteredTransform();
      await transformHold;
      return 'export const never = true;\n';
    },
  }]);
  try {
    const controller = new AbortController();
    const pending = stalled.ready(controller.signal);
    await transformEntered;
    controller.abort(new Error('stalled page load timed out'));
    await assert.rejects(() => pending, /stalled page load timed out/);
  } finally {
    releaseTransform();
    await stalled.close();
    await rm(stalledProfile, { recursive: true, force: true });
  }
});

test('loadFixture reports server and browser fetch failures on rejection and preserves evidence on abort', async () => {
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
  try {
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
  } finally {
    await cdp.close();
  }

  const stallCdp = await wsServer((send, onMessage) => {
    onMessage(message => {
      if (message.method === 'Network.enable') {
        send({ id: message.id, result: {} });
        return;
      }
      if (message.method === 'Runtime.evaluate') {
        send({ method: 'Network.requestWillBeSent', params: { requestId: '2', request: { url: 'http://127.0.0.1:1/chunks/drop.js' } } });
        send({ method: 'Network.loadingFailed', params: { requestId: '2', errorText: 'net::ERR_CONNECTION_RESET', canceled: false } });
      }
    });
  });
  try {
    const controller = new AbortController();
    let seenNetworkFailure!: () => void;
    const networkFailureSeen = new Promise<void>(done => { seenNetworkFailure = done; });
    const trackedNetworkFailures: { url: string; status?: number; reason: string }[] = [];
    const originalPush = trackedNetworkFailures.push.bind(trackedNetworkFailures);
    trackedNetworkFailures.push = (...items) => {
      const count = originalPush(...items);
      seenNetworkFailure();
      return count;
    };
    const pending = loadFixture(stallCdp.url, { fixture: '/fixture.js', failures: [] }, controller.signal, trackedNetworkFailures);
    await networkFailureSeen;
    controller.abort(new Error('fixture page load timed out'));
    await assert.rejects(
      () => pending,
      (error: unknown) => {
        assert.ok(error instanceof Error);
        assert.ok(!(error instanceof assert.AssertionError));
        assert.match(error.message, /fixture could not be loaded \(\/fixture\.js\)/);
        assert.match(error.message, /browser net::ERR_CONNECTION_RESET \(http:\/\/127\.0\.0\.1:1\/chunks\/drop\.js\)/);
        assert.match(error.message, /fixture page load timed out/);
        return true;
      },
    );
  } finally {
    await stallCdp.close();
  }

  const formatted = formatFixtureLoadError('/entry.tsx', 'TypeError: Failed to fetch', [], [{ url: 'http://127.0.0.1:1/entry.tsx', reason: 'net::ERR_EMPTY_RESPONSE' }]);
  assert.ok(!(formatted instanceof assert.AssertionError));
  assert.match(formatted.message, /fixture could not be loaded \(\/entry\.tsx\): browser net::ERR_EMPTY_RESPONSE/);
});
