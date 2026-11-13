import {test} from 'node:test';
import assert from 'node:assert/strict';
import {duration,probePath,appendTraffic,trafficWindow,trafficPath,trafficAge} from '../ui/metrics.mjs';
test('chart uses measured timestamps and leaves gaps for failed probes',()=>{
  const path=probePath([{at:100,latency_ms:1},{at:200,latency_ms:null},{at:300,latency_ms:2},{at:500,latency_ms:1}]);
  assert.equal((path.match(/M/g)||[]).length,2);assert.equal((path.match(/L/g)||[]).length,1);assert.match(path,/L600\.0,/);
  assert.equal(probePath([{at:100,latency_ms:null}]),'');assert.equal(probePath([]),'');
  assert.equal(duration(3661000),'01:01:01');assert.equal(duration(undefined),'—');
});

const rate=(at,receive_bps=2048,send_bps=1024)=>({at,receive_bps,send_bps});
test('traffic warm-up expands only real sample time and reaches a fixed two-minute window',()=>{
  const single=trafficWindow([rate(1000)]);
  assert.equal(single.span,0);assert.equal(trafficPath(single,'receive_bps',4096),'');
  const warm=trafficWindow([rate(0,null,null),rate(1000),rate(4000),rate(7000),rate(10000)]);
  assert.equal(warm.first,1000);assert.equal(warm.span,9000);
  assert.match(trafficPath(warm,'receive_bps',4096),/^M0\.0,.*L600\.0,/);
  assert.equal(trafficAge(warm.span),'9 秒前');assert.equal(trafficAge(warm.span/2),'4.5 秒前');
  const mature=trafficWindow(Array.from({length:61},(_,index)=>rate(1000+index*2000)));
  assert.equal(mature.measured.length,61);assert.equal(mature.span,120000);
  assert.equal(trafficAge(mature.span),'2 分钟前');assert.equal(trafficAge(mature.span/2),'1 分钟前');
  const sliding=trafficWindow([rate(0),rate(10000),rate(120000),rate(130000)]);
  assert.equal(sliding.first,10000);assert.equal(sliding.span,120000);
  assert.equal(trafficWindow([rate(0,null,null)]).measured.length,0);
});
test('traffic missing samples and time gaps retain measured history without false lines or zeros',()=>{
  const gap=trafficWindow([rate(1000),rate(3000),rate(5000,null,null),rate(7000),rate(9000)]);
  assert.equal(gap.first,1000);assert.deepEqual(gap.measured.map(point=>point.at),[1000,3000,7000,9000]);
  const path=trafficPath(gap,'receive_bps',4096);
  assert.equal((path.match(/M/g)||[]).length,2);assert.equal((path.match(/L/g)||[]).length,2);
  assert.doesNotMatch(path,/,78\.0/);
  const delayed=trafficWindow([rate(1000),rate(3000),rate(25000),rate(27000)]);
  assert.equal(delayed.first,1000);assert.equal((trafficPath(delayed,'receive_bps',4096).match(/M/g)||[]).length,2);
});
test('traffic fallback preserves ordinary unavailability and resets confirmed session boundaries',()=>{
  const sample=(timestamp_ms,received_bytes,state='collecting')=>({timestamp_ms,received_bytes,sent_bytes:received_bytes,receive_bps:2048,send_bps:1024,state});
  let history=appendTraffic([],sample(1000,100),true);
  history=appendTraffic(history,sample(3000,200),true);
  history=appendTraffic(history,sample(5000,0,'unavailable'),true);
  assert.equal(history.length,3);assert.equal(history.at(-1).receive_bps,null);assert.equal(history.at(-1).received_bytes,null);
  history=appendTraffic(history,sample(7000,250),true);assert.equal(history.length,4);
  history=appendTraffic(history,sample(9000,0,'unavailable'),true);
  history=appendTraffic(history,sample(11000,10),true);assert.equal(history.length,1,'Counter rollback starts a new session even across a missing sample');
  assert.deepEqual(appendTraffic(history,sample(13000,20,'disabled'),true),[]);
  assert.deepEqual(appendTraffic(history,sample(13000,20),false),[]);
  assert.equal(appendTraffic(history,sample(1000,30),true).length,1,'Timestamp rollback starts a new session');
  history=[];for(let index=0;index<70;index++)history=appendTraffic(history,sample(1000+index*2000,index*100),true);
  assert.equal(history.length,61);assert.equal(trafficWindow(history).span,120000);
});
