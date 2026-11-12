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
  if(!enabled)return [];
  if(!Number.isFinite(sample?.timestamp_ms))return history;
  const point={at:sample.timestamp_ms,receive_bps:sample.state==='collecting'?sample.receive_bps:null,send_bps:sample.state==='collecting'?sample.send_bps:null,received_bytes:sample.received_bytes,sent_bytes:sample.sent_bytes};
  const last=history.at(-1);
  if(last&&point.at<last.at)return history;
  const reset=last&&["received_bytes","sent_bytes"].some(key=>Number.isFinite(point[key])&&Number.isFinite(last[key])&&point[key]<last[key]);
  return [...(reset?[]:history.filter(item=>item.at!==point.at)),point].slice(-60);
}
