// Presentation only. Public catalog prices are supplied by the native read-only API;
// model overrides and display preferences remain in scoped local storage.
const DAY=86_400_000;
const SHANGHAI_OFFSET=8*3_600_000;
export const usagePricesKey='codex-proxy-tray:usage-prices:v1';
export const usageSettingsKey='codex-proxy-tray:usage-settings:v1';

export function formatTokens(value){
  if(!Number.isSafeInteger(value)||value<0)return '—';
  if(value===0)return '0';
  const unit=value>=1e9?['B',1e9]:value>=1e6?['M',1e6]:value>=1e3?['K',1e3]:['',1];
  return new Intl.NumberFormat('zh-CN',{maximumFractionDigits:unit[0]?2:0}).format(value/unit[1])+unit[0];
}

export function shanghaiInput(value=Date.now()){
  if(!Number.isSafeInteger(value)||value<0||value>8_640_000_000_000_000-SHANGHAI_OFFSET)return '';
  return new Date(value+SHANGHAI_OFFSET).toISOString().slice(0,16);
}

export function parseShanghaiInput(value){
  if(typeof value!=='string'||!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(value))return null;
  const parsed=Date.parse(`${value}:00+08:00`);
  return Number.isSafeInteger(parsed)&&parsed>=0&&shanghaiInput(parsed)===value?parsed:null;
}

export function usageRange(period,now=Date.now(),custom){
  if(!Number.isSafeInteger(now)||now<=0||now>8_640_000_000_000_000)throw new RangeError('用量统计时间无效。');
  if(period==='24h')return {startMs:Math.max(0,now-DAY),endMs:now};
  if(period==='all')return {startMs:0,endMs:now};
  if(period==='custom'){
    const startMs=parseShanghaiInput(custom?.start),endMs=custom?.followNow?now:parseShanghaiInput(custom?.end);
    if(startMs===null||endMs===null||startMs>=endMs||endMs>now)throw new RangeError('请填写有效的起止时间，结束时间不能晚于现在。');
    return {startMs,endMs};
  }
  const days=period==='today'?1:period==='7d'?7:period==='14d'?14:period==='30d'?30:null;
  if(!days)throw new RangeError('不支持的用量统计范围。');
  const today=Math.floor((now+SHANGHAI_OFFSET)/DAY)*DAY-SHANGHAI_OFFSET;
  return {startMs:Math.max(0,today-(days-1)*DAY),endMs:now};
}

const defaultSettings={autoScan:true,scanSeconds:20,showTrend:true,autoPrices:true,currency:'USD',chartStyle:'bar'};
export function loadUsageSettings(storage){
  try{const value=JSON.parse((storage??globalThis.localStorage)?.getItem(usageSettingsKey)||'null');return {...defaultSettings,...(typeof value?.autoScan==='boolean'?{autoScan:value.autoScan}:{}),...([5,10,20,30,60].includes(value?.scanSeconds)?{scanSeconds:value.scanSeconds}:{}),...(typeof value?.showTrend==='boolean'?{showTrend:value.showTrend}:{}),...(typeof value?.autoPrices==='boolean'?{autoPrices:value.autoPrices}:{}),...(['USD','CNY'].includes(value?.currency)?{currency:value.currency}:{}),...(['bar','area'].includes(value?.chartStyle)?{chartStyle:value.chartStyle}:{})};}catch{return {...defaultSettings};}
}
export function saveUsageSettings(settings,storage){
  const value={autoScan:settings.autoScan,scanSeconds:settings.scanSeconds,showTrend:settings.showTrend,autoPrices:settings.autoPrices??true,currency:settings.currency??'USD',chartStyle:settings.chartStyle??'bar'};
  if(typeof value.autoScan!=='boolean'||typeof value.showTrend!=='boolean'||typeof value.autoPrices!=='boolean'||![5,10,20,30,60].includes(value.scanSeconds)||!['USD','CNY'].includes(value.currency)||!['bar','area'].includes(value.chartStyle))throw Error('用量设置无效。');
  try{(storage??globalThis.localStorage).setItem(usageSettingsKey,JSON.stringify(value));}catch{throw Error('用量设置未能保存，请检查本机存储是否可用。');}
  return value;
}

export function convertUsageCost(value,currency='USD',rate=null){
  if(typeof value!=='number'||!Number.isFinite(value)||value<0)return null;
  if(currency==='USD')return value;
  if(currency!=='CNY'||typeof rate!=='number'||!Number.isFinite(rate)||rate<=0)return null;
  const converted=value*rate;return Number.isFinite(converted)&&converted>=0?converted:null;
}

export function usageAreaSegments(points,baseline){
  if(!Array.isArray(points)||!Number.isFinite(baseline))return [];
  const segments=[];let current=[];
  function flush(){
    if(!current.length)return;
    let line=`M ${current[0].x} ${current[0].y}`;
    for(let index=1;index<current.length;index++){const previous=current[index-1],point=current[index],middle=(previous.x+point.x)/2;line+=` C ${middle} ${previous.y} ${middle} ${point.y} ${point.x} ${point.y}`;}
    const area=current.length>1?`${line} L ${current.at(-1).x} ${baseline} L ${current[0].x} ${baseline} Z`:'';
    segments.push({line,area,points:current});current=[];
  }
  for(const point of points){if(!point||!Number.isFinite(point.x)||!Number.isFinite(point.y)){flush();continue;}if(current.length&&point.x<=current.at(-1).x)flush();current.push(point);}
  flush();return segments;
}

export function usageRecordEntries(records){
  if(!Array.isArray(records))return [];
  const seen=new Map(),entries=Array(records.length);
  // Count identical metadata from the oldest end, so a prepended occurrence does
  // not change the React keys of the identical rows already being displayed.
  for(let index=records.length-1;index>=0;index--){
    const row=records[index],metadata=JSON.stringify([row.at,row.model,row.provider??null,row.input_tokens,row.cached_input_tokens,row.cache_write_input_tokens??null,row.output_tokens,row.reasoning_output_tokens??null,row.total_tokens]),occurrence=seen.get(metadata)||0;
    seen.set(metadata,occurrence+1);entries[index]={row,key:`${metadata}:${occurrence}`};
  }
  return entries;
}

export function mergePrices(automatic={},overrides={}){
  const valid=prices=>Object.fromEntries(priceEntries(prices).flatMap(([model,price])=>{const parsed=validatePrice(price);return parsed?[[model,parsed]]:[];}));
  return {...valid(automatic),...valid(overrides)};
}

function numericPrice(value){
  if(typeof value==='string'){
    value=value.trim();
    if(!/^(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?$/i.test(value))return null;
    value=Number(value);
  }
  return typeof value==='number'&&Number.isFinite(value)&&value>=0?(value===0?0:value):null;
}

export function validatePrice(value){
  if(!value||typeof value!=='object'||Array.isArray(value))return null;
  const input=numericPrice(value.input),cached=numericPrice(value.cached),output=numericPrice(value.output);
  return input===null||cached===null||output===null?null:{input,cached,output};
}

function priceEntries(prices){
  if(!prices||typeof prices!=='object'||Array.isArray(prices))return [];
  return Object.entries(prices).filter(([model])=>model.length>0&&model.length<=120&&!/[\u0000-\u001f\u007f]/.test(model));
}

export function loadPrices(storage){
  try{
    const raw=(storage??globalThis.localStorage)?.getItem(usagePricesKey);
    if(!raw||raw.length>65_536)return {};
    const saved=JSON.parse(raw);
    if(saved.schema!==1||!saved.models||priceEntries(saved.models).length>256)return {};
    return Object.fromEntries(priceEntries(saved.models).flatMap(([model,price])=>{const valid=validatePrice(price);return valid?[[model,valid]]:[];}));
  }catch{return {};}
}

export function savePrices(prices,storage){
  const entries=priceEntries(prices);
  if(!prices||typeof prices!=='object'||Array.isArray(prices)||entries.length!==Object.keys(prices).length||entries.length>256)throw Error('模型单价配置无效。');
  const valid=Object.fromEntries(entries.map(([model,price])=>{const value=validatePrice(price);if(!value)throw Error('请为每个模型填写完整、非负的输入、缓存输入和输出单价。');return [model,value];}));
  const encoded=JSON.stringify({schema:1,models:valid});
  if(encoded.length>65_536)throw Error('模型单价配置超过允许范围。');
  try{
    const target=storage??globalThis.localStorage;
    if(!target||typeof target.setItem!=='function')throw Error('unavailable');
    target.setItem(usagePricesKey,encoded);
  }catch{throw Error('模型单价未能保存，请检查本机存储是否可用。');}
  return valid;
}

function tokenParts(tokens){
  if(!tokens||typeof tokens!=='object')return null;
  const input=tokens.input_tokens,cached=tokens.cached_input_tokens,output=tokens.output_tokens;
  if(![input,cached,output].every(value=>Number.isSafeInteger(value)&&value>=0)||cached>input||!Number.isSafeInteger(input+output))return null;
  if(tokens.reasoning_output_tokens!==undefined&&(!Number.isSafeInteger(tokens.reasoning_output_tokens)||tokens.reasoning_output_tokens<0||tokens.reasoning_output_tokens>output))return null;
  return {input,cached,output};
}

export function estimateCost(tokens,rawPrice){
  const count=tokenParts(tokens),price=validatePrice(rawPrice);
  if(!count||!price)return null;
  // This catalog has no cache-write rate. Keep such estimates unknown.
  if(tokens.cache_write_input_tokens!==undefined&&tokens.cache_write_input_tokens!==0)return null;
  // Cached input is already within input. Reasoning is already within output.
  const fresh=count.input-count.cached;
  const cost=fresh/1e6*price.input+count.cached/1e6*price.cached+count.output/1e6*price.output;
  if(!Number.isFinite(cost)||cost<0)return null;
  if(cost===0&&((fresh>0&&price.input>0)||(count.cached>0&&price.cached>0)||(count.output>0&&price.output>0)))return null;
  return cost;
}

function modelPrice(prices,model){return Object.hasOwn(prices,model)?prices[model]:null;}
function pricedRow(row,prices){
  const model=typeof row.model==='string'&&row.model?row.model:'unknown';
  const cost=estimateCost(row,modelPrice(prices,model));
  return {...row,estimated_cost_usd:cost,missing_price:cost===null,knownSubtotalUSD:cost,missingPricingModels:cost===null?[model]:[]};
}

function pricedGroup(group,models){
  const known=models.map(model=>model.estimated_cost_usd).filter(value=>value!==null);
  const sum=known.reduce((sum,value)=>sum+value,0);
  const knownSubtotalUSD=known.length&&Number.isFinite(sum)?sum:null;
  const missingPricingModels=[...new Set(models.flatMap(model=>model.missingPricingModels))].sort();
  const count=tokenParts(group),parts=models.map(tokenParts);
  const matches=count&&parts.every(Boolean)&&['input','cached','output'].every(key=>parts.reduce((sum,part)=>sum+part[key],0)===count[key]);
  const complete=matches&&models.length>0&&known.length===models.length&&knownSubtotalUSD!==null;
  return {...group,estimated_cost_usd:complete?knownSubtotalUSD:null,missing_price:!complete,knownSubtotalUSD,missingPricingModels,...(!matches?{cost_unavailable_reason:'model_breakdown_mismatch'}:{})};
}

export function withPrices(snapshot,rawPrices={}){
  if(!snapshot||typeof snapshot!=='object')return snapshot;
  const result=structuredClone(snapshot);
  const prices=Object.fromEntries(priceEntries(rawPrices).flatMap(([model,price])=>{const valid=validatePrice(price);return valid?[[model,valid]]:[];}));
  result.models=(Array.isArray(result.models)?result.models:[]).map(row=>pricedRow(row,prices));
  result.providers=(Array.isArray(result.providers)?result.providers:[]).map(provider=>{const models=(Array.isArray(provider.models)?provider.models:[]).map(row=>pricedRow(row,prices));return {...pricedGroup(provider,models),models};});
  result.records=(Array.isArray(result.records)?result.records:[]).map(row=>pricedRow(row,prices));
  result.summary=pricedGroup(result.summary||{},result.models);
  result.series=(Array.isArray(result.series)?result.series:[]).map(bucket=>{
    if(Array.isArray(bucket.models)&&bucket.models.length){const models=bucket.models.map(row=>pricedRow(row,prices));return {...pricedGroup(bucket,models),models};}
    // Old/foreign snapshots without a model breakdown cannot be given an average price.
    if(result.models.length===1)return pricedRow({...bucket,model:result.models[0].model},prices);
    const count=tokenParts(bucket);
    if(count&&count.input===0&&count.output===0&&result.models.length&&result.models.every(model=>!model.missing_price))return {...bucket,estimated_cost_usd:0,missing_price:false,knownSubtotalUSD:0,missingPricingModels:[]};
    return {...bucket,estimated_cost_usd:null,missing_price:true,knownSubtotalUSD:null,missingPricingModels:[...result.summary.missingPricingModels],cost_unavailable_reason:'model_breakdown_missing'};
  });
  if(result.source_available===false){
    const unavailable=row=>({...row,estimated_cost_usd:null,missing_price:true,knownSubtotalUSD:null,cost_unavailable_reason:'source_unavailable',...(Array.isArray(row.models)?{models:row.models.map(unavailable)}:{})});
    result.summary=unavailable(result.summary);
    result.series=result.series.map(unavailable);result.models=result.models.map(unavailable);result.providers=result.providers.map(unavailable);result.records=result.records.map(unavailable);
  }
  return result;
}
