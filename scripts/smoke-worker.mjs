// Exercise the compiled WASM inside workerd; no Cloudflare login or API key needed.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { once } from 'node:events';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';

let calls = 0;
let completeStream;
const first = 'event: response.output_text.delta\ndata: {"delta":"hello"}\n\n';
const last = 'event: response.completed\ndata: {}\n\n';
const mock = createServer(async (request, response) => {
  calls++;
  try {
    assert.equal(request.headers.authorization, 'Bearer test-key');
    assert.equal(request.headers.cookie, undefined);
    if (request.url === '/v1/responses?probe=1') {
      const chunks = [];
      for await (const chunk of request) chunks.push(chunk);
      assert.equal(Buffer.concat(chunks).toString(), '{"input":"hello","stream":true}');
      assert.equal(request.headers['openai-project'], 'proj_test');
      response.writeHead(200, { 'content-type': 'text/event-stream', 'x-request-id': 'req_wasm' });
      completeStream = () => response.end(last);
      response.write(first);
    } else {
      response.writeHead(429, { 'content-type': 'application/json', 'retry-after': '2' });
      response.end('{"error":{"code":"rate_limit_exceeded"}}');
    }
  } catch (error) {
    response.writeHead(500);
    response.end(error.message);
  }
});
mock.listen(0, '127.0.0.1');
await once(mock, 'listening');
const origin = `http://127.0.0.1:${mock.address().port}`;
const port = process.env.RILL_TEST_PORT || '18787';
const base = `http://127.0.0.1:${port}`;
const child = spawn(process.env.RILL_WRANGLER || 'wrangler', ['dev', '--config', 'wrangler.prebuilt.toml', '--local', '--ip', '127.0.0.1', '--port', port, '--var', `RILL_UPSTREAM:${origin}`], {
  cwd: fileURLToPath(new URL('../crates/worker', import.meta.url)),
  env: { ...process.env, CI: '1', WRANGLER_SEND_METRICS: 'false' },
  stdio: ['ignore', 'pipe', 'pipe'],
  detached: process.platform !== 'win32',
});
let log = '';
let spawnError;
child.on('error', error => { spawnError = error; });
child.stdout.on('data', value => { log = (log + value).slice(-12000); });
child.stderr.on('data', value => { log = (log + value).slice(-12000); });

try {
  let ready = false;
  for (let attempt = 0; attempt < 180; attempt++) {
    if (spawnError) throw spawnError;
    if (child.exitCode !== null) throw new Error(`Wrangler exited: ${child.exitCode}\n${log}`);
    try {
      const response = await fetch(`${base}/health`, { signal: AbortSignal.timeout(500) });
      const body = await response.json();
      if (response.ok && body.service === 'rill') { ready = true; break; }
    } catch { /* Wait for local runtime startup. */ }
    await delay(500);
  }
  assert.ok(ready, `Worker did not start\n${log}`);
  assert.equal((await fetch(`${base}/v1/models`)).status, 401);
  const llms = await fetch(`${base}/llms.txt`);
  assert.match(llms.headers.get('content-type'), /^text\/plain/);
  assert.ok((await llms.text()).includes(`Base URL: ${base}/v1`));
  const page = await fetch(`${base}/`);
  assert.match(page.headers.get('content-type'), /^text\/html/);
  assert.ok((await page.text()).includes(`${base}/v1`));
  const preflight = await fetch(`${base}/v1/responses`, { method: 'OPTIONS', headers: { 'Access-Control-Request-Headers': 'authorization, openai-project' } });
  assert.equal(preflight.status, 204);
  assert.equal(preflight.headers.get('access-control-allow-headers'), 'authorization, openai-project');
  assert.equal(calls, 0);

  const stream = await fetch(`${base}/v1/responses?probe=1`, {
    method: 'POST', signal: AbortSignal.timeout(15000),
    headers: { authorization: 'Bearer test-key', 'content-type': 'application/json', 'openai-project': 'proj_test', cookie: 'private' },
    body: '{"input":"hello","stream":true}',
  });
  assert.equal(stream.status, 200);
  assert.equal(stream.headers.get('x-request-id'), 'req_wasm');
  assert.equal(stream.headers.get('cache-control'), 'no-store');
  const reader = stream.body.getReader();
  const decoder = new TextDecoder();
  let text = '';
  while (!text.endsWith('\n\n')) {
    const chunk = await reader.read();
    assert.equal(chunk.done, false);
    text += decoder.decode(chunk.value, { stream: true });
  }
  assert.equal(text, first);
  completeStream();
  completeStream = undefined;
  while (true) {
    const chunk = await reader.read();
    if (chunk.done) break;
    text += decoder.decode(chunk.value, { stream: true });
  }
  assert.equal(text, first + last);

  const error = await fetch(`${base}/v1/models`, { headers: { authorization: 'Bearer test-key' } });
  assert.equal(error.status, 429);
  assert.equal(error.headers.get('retry-after'), '2');
  assert.equal(await error.text(), '{"error":{"code":"rate_limit_exceeded"}}');
  assert.equal(calls, 2);
  console.log('WASM/workerd smoke passed: docs, auth, CORS, payload/query/headers, incremental SSE, upstream errors.');
} catch (error) {
  console.error(log);
  throw error;
} finally {
  completeStream?.();
  mock.closeAllConnections();
  mock.close();
  if (child.exitCode === null) {
    try {
      if (process.platform === 'win32') child.kill();
      else process.kill(-child.pid, 'SIGTERM');
      await Promise.race([once(child, 'exit'), delay(2000)]);
      if (child.exitCode === null && child.signalCode === null) {
        if (process.platform === 'win32') child.kill('SIGKILL');
        else process.kill(-child.pid, 'SIGKILL');
      }
    } catch { /* Child may have exited between checks. */ }
  }
}
