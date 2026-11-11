import test from 'node:test';
import assert from 'node:assert/strict';
import {logDate,logDateRange,filterLogs} from '../ui/logs.mjs';
test('quick date ranges follow the displayed day across midnight, year and leap day boundaries',()=>{
  const now=Date.UTC(2026,11,31,16);
  assert.deepEqual(logDateRange('today',now),{start:'2027-01-01',end:'2027-01-01'});
  assert.deepEqual(logDateRange('yesterday',now),{start:'2026-12-31',end:'2026-12-31'});
  assert.deepEqual(logDateRange('week',now),{start:'2026-12-26',end:'2027-01-01'});
  assert.deepEqual(logDateRange('yesterday',Date.UTC(2028,2,1)),{start:'2028-02-29',end:'2028-02-29'});
  assert.deepEqual(logDateRange('all',now),{start:'',end:''});
});
test('log dates and inclusive date ranges use the displayed Shanghai day',()=>{
  const logs=[{id:1,at:Date.UTC(2026,10,11,15,59,59),level:'info'},{id:2,at:Date.UTC(2026,10,11,16),level:'warning'},{id:3,at:Date.UTC(2026,10,12,15,59,59),level:'error'},{id:4,at:Date.UTC(2026,10,12,16),level:'warning'}];
  assert.equal(logDate(logs[0].at),'2026-11-11');assert.equal(logDate(logs[1].at),'2026-11-12');
  assert.deepEqual(filterLogs(logs,{start:'2026-11-12',end:'2026-11-12'}).map(log=>log.id),[2,3]);
  assert.deepEqual(filterLogs(logs,{level:'warning',start:'2026-11-12',end:'2026-11-12'}).map(log=>log.id),[2]);
  assert.deepEqual(filterLogs(logs,{start:'2026-11-13',end:'2026-11-12'}),[]);assert.equal(filterLogs(logs).length,4);
  assert.deepEqual(filterLogs([{at:NaN,level:'info'}]),[]);
});
test('keyword search matches category and full details together without interpreting regular expressions',()=>{
  const logs=[{id:1,at:Date.UTC(2026,10,12),level:'warning',category:'流量',message:'采集失败',details:'Windows 操作失败（233）'},{id:2,at:Date.UTC(2026,10,12),level:'info',category:'检测',message:'检测完成',details:'耗时 233 ms'}];
  assert.deepEqual(filterLogs(logs,{query:'  WINDOWS 233 '}).map(log=>log.id),[1]);
  assert.deepEqual(filterLogs(logs,{query:'流量 失败',level:'warning'}).map(log=>log.id),[1]);
  assert.deepEqual(filterLogs(logs,{query:'233',level:'info'}).map(log=>log.id),[2]);
  assert.deepEqual(filterLogs(logs,{query:'.*'}),[]);
});
