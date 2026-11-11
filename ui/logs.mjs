const dateFormat=new Intl.DateTimeFormat('en-CA',{timeZone:'Asia/Shanghai',year:'numeric',month:'2-digit',day:'2-digit'});
const timeFormat=new Intl.DateTimeFormat('zh-CN',{timeZone:'Asia/Shanghai',hour12:false,hour:'2-digit',minute:'2-digit',second:'2-digit'});
export function logDate(at){
  if(!Number.isFinite(at)||!Number.isFinite(new Date(at).getTime()))return '';
  const parts=Object.fromEntries(dateFormat.formatToParts(new Date(at)).map(part=>[part.type,part.value]));
  return `${parts.year}-${parts.month}-${parts.day}`;
}
export const logTime=at=>timeFormat.format(new Date(at));
export function logDateRange(preset,now=Date.now()){
  if(!['today','yesterday','week'].includes(preset))return {start:'',end:''};
  const end=logDate(now-(preset==='yesterday'?86400000:0));
  return {start:preset==='week'?logDate(now-6*86400000):end,end};
}
export function filterLogs(logs,{level='all',start='',end='',query=''}={}){
  const words=query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  return logs.filter(log=>{const date=logDate(log.at),text=[log.message,log.category,log.details].filter(Boolean).join(' ').toLocaleLowerCase();return date&&(level==='all'||log.level===level)&&(!start||date>=start)&&(!end||date<=end)&&words.every(word=>text.includes(word));});
}
