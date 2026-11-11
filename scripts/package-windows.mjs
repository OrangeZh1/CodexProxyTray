// The GNU WebView2 runtime requires its x64 loader beside both executables.
import { createReadStream } from 'node:fs';
import { copyFile, mkdir, open, readdir, writeFile, readFile } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { createHash } from 'node:crypto';
const options = Object.fromEntries(process.argv.slice(2).reduce((pairs,item,index,args) => index % 2 ? pairs : [...pairs,[item,args[index+1]]],[]));
for (const key of ['--bin-dir','--loader','--out']) if (!options[key]) throw Error(`Required: ${key}`);
const source = resolve(options['--bin-dir']), loader = resolve(options['--loader']), output = resolve(options['--out']);
const {version} = JSON.parse(await readFile(new URL('../package.json',import.meta.url),'utf8'));
async function sha256(path) { const hash=createHash('sha256');for await(const bytes of createReadStream(path)) hash.update(bytes);return hash.digest('hex'); }
async function x64(path) {
  const file=await open(path,'r');
  try { const head=Buffer.alloc(64);await file.read(head,0,64,0);if(head.readUInt16LE(0)!==0x5a4d)throw Error(`Not a PE: ${path}`);const pe=Buffer.alloc(6);await file.read(pe,0,6,head.readUInt32LE(60));if(pe.readUInt32LE(0)!==0x4550||pe.readUInt16LE(4)!==0x8664)throw Error(`Not Windows x64: ${path}`); }
  finally { await file.close(); }
}
const files=[['CodexProxyTray.exe',join(source,'CodexProxyTray.exe')],['codex-proxy-cli.exe',join(source,'codex-proxy-cli.exe')],['WebView2Loader.dll',loader]];
for(const [,path] of files) await x64(path);
if(await sha256(loader)!=='86545b66cdb0603bc26b626fb9ad610cb6e71f28d468f5ea66df23b03dda96d5')throw Error('WebView2Loader.dll differs from the verified webview2-com-sys 0.39.1 x64 loader.');
await mkdir(output,{recursive:true});if((await readdir(output)).length)throw Error('Output must be empty; existing deliverables are preserved.');
const receipt={schema:1,product:'CodexProxyTray',version,packaged_at:new Date().toISOString(),source_commit:process.env.GITHUB_SHA||null,files:[]};
for(const [name,path] of files){await copyFile(path,join(output,name));receipt.files.push({name,sha256:await sha256(join(output,name))});}
await writeFile(join(output,'SHA256SUMS.txt'),receipt.files.map(file=>`${file.sha256}  ${file.name}`).join('\n')+'\n');
await writeFile(join(output,'package-receipt.json'),JSON.stringify(receipt,null,2)+'\n');
console.log(`Packaged all three required runtime files: ${output}`);
