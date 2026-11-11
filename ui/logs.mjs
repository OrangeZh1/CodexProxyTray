const dateFormat=new Intl.DateTimeFormat('en-CA',{timeZone:'Asia/Shanghai',year:'numeric',month:'2-digit',day:'2-digit'});
const timeFormat=new Intl.DateTimeFormat('zh-CN',{timeZone:'Asia/Shanghai',hour12:false,hour:'2-digit',minute:'2-digit',second:'2-digit'});
export function logDate(at){
  if(!Number.isFinite(at)||!Number.isFinite(new Date(at).getTime()))return '';
  const parts=Object.fromEntries(dateFormat.formatToParts(new Date(at)).map(part=>[part.type,part.value]));
  return `${parts.year}-${parts.month}-${parts.day}`;
}
export const logTime=at=>timeFormat.format(new Date(at));
export function filterLogs(logs,{level='all',start='',end=''}={}){
  return logs.filter(log=>{const date=logDate(log.at);return date&&(level==='all'||log.level===level)&&(!start||date>=start)&&(!end||date<=end);});
}
