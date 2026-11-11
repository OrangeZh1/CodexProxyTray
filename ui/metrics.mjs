export function duration(milliseconds) {
  if (!Number.isFinite(milliseconds) || milliseconds < 0) return '—';
  const seconds=Math.floor(milliseconds/1000), hours=Math.floor(seconds/3600);
  return [hours,Math.floor(seconds%3600/60),seconds%60].map(value=>String(value).padStart(2,'0')).join(':');
}
export function probePath(samples,width=600,height=150) {
  const points=samples.filter(sample=>Number.isFinite(sample.at));
  const values=points.map(sample=>sample.latency_ms).filter(Number.isFinite);
  if (!values.length) return '';
  const first=points[0].at, span=Math.max(1,points.at(-1).at-first), maximum=Math.max(1,...values)*1.2;
  let drawing=false;
  return points.map(point=>{
    if(!Number.isFinite(point.latency_ms)){drawing=false;return '';}
    const command=drawing?'L':'M';drawing=true;
    return `${command}${((point.at-first)/span*width).toFixed(1)},${(height-Math.min(maximum,point.latency_ms)/maximum*(height-12)).toFixed(1)}`;
  }).filter(Boolean).join(' ');
}
