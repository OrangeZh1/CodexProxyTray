import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseManifest, createUpdateCheck } from '../scripts/update-check.mjs';
test('public update manifest rejects the legacy launcher and malformed versions', () => {
  assert.throws(() => parseManifest({schema:1,product:'chatgpt-proxy-launcher',version:'1.0.0',notes:''}));
  for (const version of ['01.0.0','1.0','v0.2.0-beta','4294967296.0.0']) assert.throws(() => parseManifest({schema:1,product:'CodexProxyTray',version,notes:''}));
  assert.equal(parseManifest({schema:1,product:'CodexProxyTray',version:'0.10.0',notes:'新版'}).status,'available');
});
test('missing branded asset is unpublished; concurrent and repeated checks share cache', async () => {
  let calls=0,now=0; const check=createUpdateCheck(async()=>{calls++;return new Response('',{status:404});},()=>now);
  assert.equal((await Promise.all([check(),check()]))[0].status,'unpublished'); await check(); assert.equal(calls,1);
  now=900001;await check();assert.equal(calls,2);
});
test('failures are friendly and cached briefly, invalid or oversized payloads rejected', async () => {
  let calls=0,now=0;const check=createUpdateCheck(async()=>{calls++;return new Response('',{status:403});},()=>now);
  await assert.rejects(check(),/暂时限制/);await assert.rejects(check(),/暂时限制/);assert.equal(calls,1);now=60001;await assert.rejects(check());assert.equal(calls,2);
  await assert.rejects(createUpdateCheck(async()=>new Response('x'.repeat(65537)))(),/发布信息无效/);
  await assert.rejects(createUpdateCheck(async()=>new Response('<html>'))(),/发布信息无效/);
});
