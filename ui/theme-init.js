// Synchronous, local-only first paint. Keep the preference contract in themes.mjs.
(() => {
  let mode='system';
  try {const saved=localStorage.getItem('color-theme');if(['system','light','dark'].includes(saved))mode=saved;}catch{}
  let systemDark=false;
  try {systemDark=!!window.matchMedia?.('(prefers-color-scheme: dark)').matches;}catch{}
  const resolved=mode==='dark'||mode==='system'&&systemDark?'dark':'light';
  document.documentElement.dataset.theme=resolved;
  document.documentElement.style.colorScheme=resolved;
  let reported=false;
  window.reportUiError=error=>{
    if(reported)return;reported=true;
    const message=String(error?.message||error||'未知界面错误').slice(0,2000);
    const invoke=window.__TAURI__?.core?.invoke;
    invoke?.('report_ui_error',{message}).catch(()=>{});
    const show=()=>{
      const root=document.getElementById('root');if(!root)return;
      const panel=document.createElement('div');panel.className='startup-error';
      const title=document.createElement('h1');title.textContent='界面加载失败';
      const detail=document.createElement('p');detail.textContent=message;
      const retry=document.createElement('button');retry.textContent='重新加载界面';retry.onclick=()=>window.location.reload();
      panel.append(title,detail,retry);root.replaceChildren(panel);
      invoke?.('frontend_ready').catch(()=>{});
    };
    if(document.readyState==='loading')document.addEventListener('DOMContentLoaded',show,{once:true});else show();
  };
  window.addEventListener('error',event=>window.reportUiError(event.error||event.message||'界面资源加载失败'));
  window.addEventListener('unhandledrejection',event=>window.reportUiError(event.reason));
})();
