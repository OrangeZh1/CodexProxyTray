export function duration(milliseconds) {
  if (!Number.isFinite(milliseconds) || milliseconds < 0) return '—';
  const seconds=Math.floor(milliseconds/1000), hours=Math.floor(seconds/3600);
  return [hours,Math.floor(seconds%3600/60),seconds%60].map(value=>String(value).padStart(2,'0')).join(':');
}
export function probeScale(samples) {
  const peak=Math.max(0,...samples.map(sample=>sample.latency_ms).filter(value=>Number.isFinite(value)&&value>=0));
  // A zero baseline and at least 10 ms keep sub-ms TCP jitter in proportion.
  const target=Math.max(10,peak*1.2),power=10**Math.floor(Math.log10(target));
  return [1,2,5,10].map(step=>step*power).find(value=>value>=target);
}
export function probePath(samples,width=600,height=80,maximum=probeScale(samples),maxGap=Infinity) {
  const points=samples.filter(sample=>Number.isFinite(sample.at));
  const values=points.map(sample=>sample.latency_ms).filter(Number.isFinite);
  if (!values.length) return '';
  const first=points[0].at, span=Math.max(1,points.at(-1).at-first);
  let drawing=false,previousAt=null;
  return points.map(point=>{
    if(!Number.isFinite(point.latency_ms)||point.latency_ms<0){drawing=false;return '';}
    const command=drawing&&point.at-previousAt<=maxGap?'L':'M';drawing=true;previousAt=point.at;
    return `${command}${((point.at-first)/span*width).toFixed(1)},${(height-2-Math.min(maximum,point.latency_ms)/maximum*(height-4)).toFixed(1)}`;
  }).filter(Boolean).join(' ');
}

export function appendTraffic(history,sample,enabled) {
  if(!enabled||sample?.state==='disabled')return [];
  if(!Number.isFinite(sample?.timestamp_ms))return history;
  const point={at:sample.timestamp_ms,receive_bps:sample.state==='collecting'?sample.receive_bps:null,send_bps:sample.state==='collecting'?sample.send_bps:null,received_bytes:sample.state==='collecting'?sample.received_bytes:null,sent_bytes:sample.state==='collecting'?sample.sent_bytes:null};
  const last=history.at(-1);
  const reset=(last&&point.at<last.at)||["received_bytes","sent_bytes"].some(key=>{
    const previous=history.findLast(item=>Number.isFinite(item[key]));
    return previous&&Number.isFinite(point[key])&&point[key]<previous[key];
  });
  return [...(reset?[]:history.filter(item=>item.at!==point.at)),point].slice(-61);
}

const measuredTraffic=point=>['receive_bps','send_bps'].some(key=>Number.isFinite(point[key])&&point[key]>=0);
export function trafficWindow(history,maximumSpan=120_000) {
  const samples=history.filter(point=>Number.isFinite(point.at)),start=samples.find(measuredTraffic);
  if(!start)return {first:0,last:0,span:0,points:[],measured:[]};
  const last=samples.at(-1).at,first=Math.max(start.at,last-maximumSpan);
  const points=samples.filter(point=>point.at>=first&&point.at<=last);
  return {first,last,span:last-first,points,measured:points.filter(measuredTraffic)};
}
export function trafficPath(window,key,maximum) {
  if(window.measured.length<2||window.span<=0)return '';
  return probePath([{at:window.first,latency_ms:null},...window.points.map(point=>({at:point.at,latency_ms:point[key]})),{at:window.last,latency_ms:null}],600,80,maximum,10_000);
}
export function trafficAge(milliseconds) {
  if(milliseconds<=0)return '现在';
  return milliseconds>=60_000?`${Number((milliseconds/60_000).toFixed(1))} 分钟前`:`${Number((milliseconds/1000).toFixed(1))} 秒前`;
}
