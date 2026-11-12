import {test} from 'node:test';
import assert from 'node:assert/strict';
import {formatTokens,usageRange,shanghaiInput,parseShanghaiInput,validatePrice,loadPrices,savePrices,estimateCost,withPrices,usagePricesKey,mergePrices,loadUsageSettings,saveUsageSettings,usageSettingsKey,convertUsageCost,usageAreaSegments,usageRecordEntries} from '../ui/usage.mjs';

const tokens=(input,cached,output)=>({input_tokens:input,cached_input_tokens:cached,output_tokens:output,reasoning_output_tokens:Math.min(10,output),total_tokens:input+output});
const near=(actual,expected)=>assert.ok(Math.abs(actual-expected)<1e-12,`${actual} != ${expected}`);

test('display and date ranges use real values and Shanghai calendar boundaries',()=>{
  assert.equal(formatTokens(null),'—');assert.equal(formatTokens(Infinity),'—');assert.equal(formatTokens(-1),'—');assert.equal(formatTokens(0),'0');assert.equal(formatTokens(1234),'1.23K');assert.equal(formatTokens(1_250_000),'1.25M');
  const now=Date.parse('2026-11-12T10:30:00Z');
  assert.deepEqual(usageRange('24h',now),{startMs:now-86_400_000,endMs:now});
  assert.deepEqual(usageRange('today',now),{startMs:Date.parse('2026-11-11T16:00:00Z'),endMs:now});
  assert.deepEqual(usageRange('all',now),{startMs:0,endMs:now});
  assert.deepEqual(usageRange('7d',now),{startMs:Date.parse('2026-11-05T16:00:00Z'),endMs:now});
  assert.deepEqual(usageRange('14d',now),{startMs:Date.parse('2026-10-29T16:00:00Z'),endMs:now});
  assert.deepEqual(usageRange('30d',now),{startMs:Date.parse('2026-10-13T16:00:00Z'),endMs:now});
  for(const invalid of ['unknown','__proto__','constructor'])assert.throws(()=>usageRange(invalid,now),RangeError);assert.throws(()=>usageRange('24h',NaN),RangeError);
});

test('custom date and time values retain Shanghai meaning and reject normalized invalid dates',()=>{
  const now=Date.parse('2026-11-12T10:30:00Z');
  assert.equal(shanghaiInput(now),'2026-11-12T18:30');
  assert.equal(parseShanghaiInput('2026-11-12T18:30'),now);
  for(const value of ['2026-02-30T10:00','2026-11-12T24:00','2026-11-12','2026-11-12T18:30:00','bad'])assert.equal(parseShanghaiInput(value),null);
  const custom={start:'2026-10-01T09:15',end:'2026-11-10T15:45',followNow:false};
  assert.deepEqual(usageRange('custom',now,custom),{startMs:Date.parse('2026-10-01T01:15:00Z'),endMs:Date.parse('2026-11-10T07:45:00Z')});
  assert.equal(usageRange('custom',now,{...custom,followNow:true}).endMs,now);
  for(const invalid of [{...custom,start:custom.end},{...custom,end:'2026-11-13T10:00'},{...custom,start:'bad'}])assert.throws(()=>usageRange('custom',now,invalid),RangeError);
});

test('automatic catalog rates are validated and complete local overrides take precedence',()=>{
  const automatic={a:{input:2,cached:.2,output:8},b:{input:4,output:12},c:{input:0,cached:0,output:0}},overrides={a:{input:3,cached:.3,output:10},missing:{input:1,cached:'.1',output:5}};
  assert.deepEqual(mergePrices(automatic,overrides),{a:{input:3,cached:.3,output:10},c:{input:0,cached:0,output:0},missing:{input:1,cached:.1,output:5}});
  assert.deepEqual(mergePrices(automatic,{}).a,automatic.a);
  assert.equal(estimateCost({...tokens(1_000_000,0,0),cache_write_input_tokens:100},{input:2,cached:.2,output:8}),null);
});

test('display preferences are scoped, bounded and retain defaults when storage is unavailable',()=>{
  assert.deepEqual(loadUsageSettings({getItem:()=>'{broken'}),{autoScan:true,scanSeconds:20,showTrend:true,autoPrices:true,currency:'USD',chartStyle:'bar'});
  assert.deepEqual(loadUsageSettings({getItem:()=>JSON.stringify({autoScan:false,scanSeconds:1,showTrend:false,chartStyle:'pie'})}),{autoScan:false,scanSeconds:20,showTrend:false,autoPrices:true,currency:'USD',chartStyle:'bar'});
  let encoded;const storage={setItem:(key,value)=>{assert.equal(key,usageSettingsKey);encoded=value;},getItem:()=>encoded};
  const settings={autoScan:false,scanSeconds:60,showTrend:false,autoPrices:false,currency:'CNY',chartStyle:'area'};saveUsageSettings(settings,storage);assert.deepEqual(loadUsageSettings(storage),settings);
  for(const scanSeconds of [5,30]){saveUsageSettings({...settings,scanSeconds},storage);assert.equal(loadUsageSettings(storage).scanSeconds,scanSeconds);}
  assert.throws(()=>saveUsageSettings({...settings,scanSeconds:2},storage),/无效/);
  assert.throws(()=>saveUsageSettings({...settings,autoPrices:'false'},storage),/无效/);
  assert.throws(()=>saveUsageSettings({...settings,currency:'EUR'},storage),/无效/);
  assert.throws(()=>saveUsageSettings({...settings,chartStyle:'pie'},storage),/无效/);
  assert.throws(()=>saveUsageSettings(settings,{setItem:()=>{throw Error('full');}}),/未能保存/);
});

test('area charts keep unknown intervals separate and never fill a singleton gap',()=>{
  const points=[{x:0,y:40},{x:10,y:20},null,{x:30,y:50},{x:40,y:30}],copy=structuredClone(points),segments=usageAreaSegments(points,80);
  assert.deepEqual(segments.map(segment=>segment.points.map(point=>point.x)),[[0,10],[30,40]]);assert.equal(segments.length,2);
  for(const segment of segments){assert.ok(segment.line.includes(' C '));assert.ok(segment.area.endsWith(' Z'));assert.doesNotMatch(segment.line,/NaN|Infinity/);}
  assert.deepEqual(points,copy);const separated=usageAreaSegments([{x:1,y:20},{x:2,y:NaN},{x:3,y:30}],80);assert.equal(separated.length,2);assert.ok(separated.every(segment=>segment.area===''));assert.equal(usageAreaSegments([{x:2,y:20},{x:1,y:30}],80).length,2);
  assert.deepEqual(usageAreaSegments(null,80),[]);assert.deepEqual(usageAreaSegments(points,NaN),[]);
});

test('record keys retain metadata identity across prepends and price or FX formatting changes',()=>{
  const a={at:200,model:'model-a',provider:'provider-a',...tokens(100,20,50)},b={at:100,model:'model-b',provider:null,...tokens(200,40,100)},records=[a,b],original=structuredClone(records),keys=usageRecordEntries(records).map(entry=>entry.key);
  const formatted=records.map(row=>({...row,estimated_cost_usd:99,display_currency:'CNY',duration_ms:2000,ttft_ms:300,output_tokens_per_second:30,http_status:200}));assert.deepEqual(usageRecordEntries(formatted).map(entry=>entry.key),keys);
  assert.deepEqual(usageRecordEntries([{...a,at:300},...records]).slice(1).map(entry=>entry.key),keys);
  const duplicated=[a,{...a},b],oldKeys=usageRecordEntries(duplicated).map(entry=>entry.key),prepended=usageRecordEntries([{...a},...duplicated]);assert.deepEqual(prepended.slice(1).map(entry=>entry.key),oldKeys);assert.equal(new Set(prepended.map(entry=>entry.key)).size,4);assert.deepEqual(records,original);assert.deepEqual(usageRecordEntries(null),[]);
});

test('CNY presentation uses a valid positive reference rate without changing raw USD costs',()=>{
  const raw=1.36;assert.equal(convertUsageCost(raw,'USD'),raw);near(convertUsageCost(raw,'CNY',7.25),9.86);assert.equal(raw,1.36);
  for(const rate of [null,undefined,0,-1,Infinity,NaN,'7.25'])assert.equal(convertUsageCost(raw,'CNY',rate),null);
  for(const cost of [null,undefined,-1,Infinity,NaN])assert.equal(convertUsageCost(cost,'CNY',7.25),null);
  assert.equal(convertUsageCost(0,'CNY',7.25),0);assert.equal(convertUsageCost(raw,'EUR',7.25),null);assert.equal(convertUsageCost(1e308,'CNY',100),null);
});

test('prices are complete, finite, nonnegative and preserved only in scoped local storage',()=>{
  assert.deepEqual(validatePrice({input:' 2.5 ',cached:'.25',output:'10'}),{input:2.5,cached:.25,output:10});
  assert.deepEqual(validatePrice({input:'0',cached:0,output:'0.00'}),{input:0,cached:0,output:0});
  for(const value of ['',undefined,null,-1,Infinity,NaN,true,'-1','Infinity','0x10'])assert.equal(validatePrice({input:value,cached:0,output:0}),null);
  let saved;const storage={getItem:key=>key===usagePricesKey?saved:null,setItem:(key,value)=>{assert.equal(key,usagePricesKey);saved=value;}};
  const prices={'model-a':{input:'2',cached:'.2',output:'8'}};savePrices(prices,storage);
  assert.deepEqual(loadPrices(storage),{'model-a':{input:2,cached:.2,output:8}});
  const previous=saved;assert.throws(()=>savePrices({'model-a':{input:'',cached:0,output:0}},storage));assert.equal(saved,previous);
  assert.deepEqual(loadPrices({getItem:()=>'{broken'}),{});assert.deepEqual(loadPrices({getItem:()=>{throw Error('denied');}}),{});
  assert.throws(()=>savePrices(prices,{setItem:()=>{throw Error('full');}}),/未能保存/);
});

test('cost subtracts cached input once and never charges reasoning a second time',()=>{
  const usage=tokens(1_000_000,800_000,100_000);usage.reasoning_output_tokens=90_000;
  near(estimateCost(usage,{input:2,cached:.2,output:8}),1.36);
  assert.equal(estimateCost(usage,null),null);assert.equal(estimateCost(usage,{input:2,cached:'',output:8}),null);
  assert.equal(estimateCost(tokens(10,11,1),{input:2,cached:.2,output:8}),null);
  assert.equal(estimateCost({input_tokens:10,output_tokens:1},{input:2,cached:.2,output:8}),null);
  assert.equal(estimateCost(tokens(0,0,0),{input:0,cached:0,output:0}),0);
  assert.equal(estimateCost(tokens(1_000_000,0,1_000_000),{input:1e308,cached:0,output:1e308}),null);
});

test('mixed-model groups and buckets get exact custom estimates without mutating raw data',()=>{
  const a={model:'model-a',...tokens(1_000_000,800_000,100_000),request_count:1};
  const b={model:'model-b',...tokens(500_000,100_000,50_000),request_count:1};
  const snapshot={source_available:true,summary:{...tokens(1_500_000,900_000,150_000),request_count:2},models:[a,b],providers:[{provider:'provider-a',...a,models:[a]},{provider:'provider-b',...b,models:[b]}],series:[{start_ms:0,end_ms:1,...tokens(1_500_000,900_000,150_000),models:[a,b]}],records:[{at:0,...a},{at:0,...b}]};
  const raw=structuredClone(snapshot),prices={'model-a':{input:2,cached:.2,output:8},'model-b':{input:4,cached:.4,output:12}};
  const result=withPrices(snapshot,prices);
  near(result.models[0].estimated_cost_usd,1.36);near(result.models[1].estimated_cost_usd,2.24);
  near(result.summary.estimated_cost_usd,3.6);near(result.series[0].estimated_cost_usd,3.6);
  near(result.providers[0].estimated_cost_usd,1.36);near(result.providers[1].estimated_cost_usd,2.24);
  assert.equal(result.summary.missing_price,false);assert.deepEqual(result.summary.missingPricingModels,[]);
  assert.deepEqual(snapshot,raw);assert.notEqual(result.records,snapshot.records);
  const unavailable=withPrices({...snapshot,source_available:false},prices);assert.equal(unavailable.providers[0].estimated_cost_usd,null);assert.equal(unavailable.providers[0].models[0].estimated_cost_usd,null);
});

test('pricing preserves reported and unknown performance metadata without inventing a status',()=>{
  const row={model:'model-a',...tokens(1000,200,50)};
  const measured={...row,at:10000,duration_ms:2000,ttft_ms:300,output_tokens_per_second:50/1.7,timing_estimated:true,timing_source:'codex_rollout_estimate',http_status:200};
  const unknown={...row,at:9000,duration_ms:null,ttft_ms:null,output_tokens_per_second:null,timing_estimated:false,timing_source:null,http_status:null};
  const data=withPrices({source_available:true,summary:row,models:[row],records:[measured,unknown],series:[]},{'model-a':{input:2,cached:.2,output:8}});
  for(const key of ['duration_ms','ttft_ms','output_tokens_per_second','timing_estimated','timing_source','http_status']){assert.equal(data.records[0][key],measured[key]);assert.equal(data.records[1][key],unknown[key]);}
  assert.equal(data.records[1].http_status,null);
});

test('missing rates keep totals unknown while retaining an explicitly known subtotal',()=>{
  const a={model:'priced',...tokens(1_000_000,0,0)},b={model:'missing',...tokens(1_000_000,0,0)};
  const snapshot={source_available:true,summary:tokens(2_000_000,0,0),models:[a,b],series:[{...tokens(2_000_000,0,0),models:[a,b]}],records:[a,b]};
  const result=withPrices(snapshot,{priced:{input:2,cached:0,output:0}});
  assert.equal(result.summary.estimated_cost_usd,null);assert.equal(result.summary.knownSubtotalUSD,2);assert.deepEqual(result.summary.missingPricingModels,['missing']);assert.equal(result.series[0].estimated_cost_usd,null);assert.equal(result.records[1].estimated_cost_usd,null);
  const none=withPrices(snapshot,{});assert.equal(none.summary.estimated_cost_usd,null);assert.equal(none.summary.knownSubtotalUSD,null);
  const noBreakdown=withPrices({...snapshot,series:[tokens(2_000_000,0,0)]},{priced:{input:2,cached:0,output:0},missing:{input:4,cached:0,output:0}});
  assert.equal(noBreakdown.series[0].estimated_cost_usd,null);assert.equal(noBreakdown.series[0].cost_unavailable_reason,'model_breakdown_missing');
  const mismatched=withPrices({...snapshot,summary:tokens(3_000_000,0,0)},{priced:{input:2,cached:0,output:0},missing:{input:4,cached:0,output:0}});assert.equal(mismatched.summary.estimated_cost_usd,null);assert.equal(mismatched.summary.cost_unavailable_reason,'model_breakdown_mismatch');
  const absent=withPrices({...snapshot,source_available:false},{priced:{input:0,cached:0,output:0},missing:{input:0,cached:0,output:0}});assert.equal(absent.summary.estimated_cost_usd,null);
});
