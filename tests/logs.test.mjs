import test from 'node:test';
import assert from 'node:assert/strict';
import {logDate,filterLogs} from '../ui/logs.mjs';
test('log dates and inclusive date ranges use the displayed Shanghai day',()=>{
  const logs=[{id:1,at:Date.UTC(2026,10,11,15,59,59),level:'info'},{id:2,at:Date.UTC(2026,10,11,16),level:'warning'},{id:3,at:Date.UTC(2026,10,12,15,59,59),level:'error'},{id:4,at:Date.UTC(2026,10,12,16),level:'warning'}];
  assert.equal(logDate(logs[0].at),'2026-11-11');assert.equal(logDate(logs[1].at),'2026-11-12');
  assert.deepEqual(filterLogs(logs,{start:'2026-11-12',end:'2026-11-12'}).map(log=>log.id),[2,3]);
  assert.deepEqual(filterLogs(logs,{level:'warning',start:'2026-11-12',end:'2026-11-12'}).map(log=>log.id),[2]);
  assert.deepEqual(filterLogs(logs,{start:'2026-11-13',end:'2026-11-12'}),[]);assert.equal(filterLogs(logs).length,4);
  assert.deepEqual(filterLogs([{at:NaN,level:'info'}]),[]);
});
