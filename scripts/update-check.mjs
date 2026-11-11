import { readFileSync } from 'node:fs';
export const RELEASES = 'https://github.com/OrangeZh1/chatgpt-proxy-launcher/releases';
const manifestUrl = `${RELEASES}/latest/download/CodexProxyTray-update.json`;
const { version: current } = JSON.parse(readFileSync(new URL('../package.json',import.meta.url),'utf8'));
export function parseManifest(value) {
  const parts = text => typeof text === 'string' && /^v?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(text) ? text.replace(/^v/,'').split('.').map(Number) : null;
  const latest = parts(value?.version);
  if (value?.schema !== 1 || value?.product !== 'CodexProxyTray' || typeof value.notes !== 'string' || !latest || latest.some(n => n > 0xffffffff)) throw Error('发布信息与当前应用不匹配，请查看 GitHub 发布页。');
  const installed = parts(current);
  const first = latest.findIndex((n,i) => n !== installed[i]);
  return {status:first >= 0 && latest[first] > installed[first] ? 'available' : 'current',current,latest:value.version,url:RELEASES,notes:value.notes.slice(0,16000)};
}
export function createUpdateCheck(fetcher = fetch, clock = Date.now) {
  let pending, cached;
  return () => {
    if (cached && clock() - cached.at < (cached.error ? 60000 : 900000)) return cached.error ? Promise.reject(cached.error) : Promise.resolve(cached.value);
    pending ??= (async () => {
      try {
        let response;
        try { response = await fetcher(manifestUrl, { signal: AbortSignal.timeout(12000), headers: { 'User-Agent': `CodexProxyTray/${current}` } }); }
        catch { throw Error('无法连接 GitHub，请检查本地代理后重试。'); }
        let value;
        if (response.status === 404) value = {status:'unpublished',current,latest:null,url:RELEASES,notes:'尚未发布 CodexProxyTray 正式安装包。'};
        else {
          if ([403,429].includes(response.status)) throw Error('GitHub 暂时限制访问，请稍后重试。');
          if (!response.ok) throw Error('GitHub 暂不可用，请稍后重试。');
          let bytes = 0, text = '';
          const decoder = new TextDecoder();
          for await (const chunk of response.body) { bytes += chunk.byteLength; if (bytes > 65536) throw Error('发布信息无效，请查看 GitHub 发布页。'); text += decoder.decode(chunk,{stream:true}); }
          try { value = parseManifest(JSON.parse(text + decoder.decode())); }
          catch { throw Error('发布信息无效，请查看 GitHub 发布页。'); }
        }
        cached = {at:clock(),value}; return value;
      } catch (error) { cached = {at:clock(),error}; throw error; }
    })().finally(() => { pending = undefined; });
    return pending;
  };
}
