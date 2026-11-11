import { build } from 'esbuild';
import { readFile, mkdir, copyFile, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
process.env.GOMAXPROCS ??= '1';
process.env.GOMEMLIMIT ??= '512MiB';
export const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const { version } = JSON.parse(await readFile(resolve(root,'package.json'),'utf8'));
const native = JSON.parse(await readFile(resolve(root,'src-tauri/tauri.conf.json'),'utf8'));
const cargo = await readFile(resolve(root,'src-tauri/Cargo.toml'),'utf8');
if (native.version !== version || /^version\s*=\s*"([^"]+)"/m.exec(cargo)?.[1] !== version) throw Error('UI, Tauri and Cargo versions must match.');
const fluent = JSON.parse(await readFile(resolve(root, 'node_modules/@iconify-json/fluent/icons.json'), 'utf8'));
const names = ['settings-24-regular','text-bullet-list-ltr-24-regular','info-24-regular','shield-checkmark-24-regular','plug-connected-24-regular','desktop-24-regular','window-console-20-regular','power-24-regular','open-24-regular','arrow-sync-24-regular','save-24-regular','checkmark-circle-24-regular','warning-24-regular','dismiss-24-regular','subtract-24-regular','maximize-24-regular','square-multiple-24-regular','navigation-24-regular','grid-24-regular','pulse-24-regular','clock-24-regular','data-line-24-regular','chevron-down-24-regular','chevron-right-20-regular','pause-24-regular','play-24-regular'];
const icons = Object.fromEntries(names.map(name => {
  if (!fluent.icons[name]) throw Error(`Missing Iconify Fluent icon: ${name}`);
  return [name, { width: fluent.width, height: fluent.height, ...fluent.icons[name] }];
}));
export const iconsPlugin = { name: 'local-iconify-fluent', setup(b) {
  b.onResolve({ filter: /^#fluent-icons$/ }, () => ({ path: 'fluent-icons', namespace: 'local' }));
  b.onLoad({ filter: /.*/, namespace: 'local' }, () => ({ contents: `export default ${JSON.stringify(icons)}`, loader: 'js' }));
} };
export async function buildUI() {
  await mkdir(resolve(root, 'dist'), { recursive: true });
  await build({ absWorkingDir: root, entryPoints: ['ui/app.jsx'], outfile: 'dist/app.js', bundle: true, format: 'esm', target: ['chrome109'], minify: true, jsx: 'automatic', define: { 'process.env.NODE_ENV': '"production"', '__APP_VERSION__': JSON.stringify(version) }, plugins: [iconsPlugin] });
  const [script,styles,template,boot]=await Promise.all([readFile(resolve(root,'dist/app.js')),readFile(resolve(root,'dist/app.css')),readFile(resolve(root,'ui/index.html'),'utf8'),readFile(resolve(root,'ui/theme-init.js'))]);
  const assetVersion=createHash('sha256').update(script).update('\0').update(styles).update('\0').update(boot).digest('hex').slice(0,16);
  const html=template.replace('src="app.js"',`src="app.js?v=${assetVersion}"`).replace('href="app.css"',`href="app.css?v=${assetVersion}"`).replace('src="theme-init.js"',`src="theme-init.js?v=${assetVersion}"`);
  await Promise.all([writeFile(resolve(root,'dist/index.html'),html),copyFile(resolve(root,'ui/icon.svg'),resolve(root,'dist/icon.svg')),copyFile(resolve(root,'ui/theme-init.js'),resolve(root,'dist/theme-init.js'))]);
  await writeFile(resolve(root,'dist/tray.html'),html.replace('<title>CodexProxyTray</title>','<title>CodexProxyTray · 托盘菜单</title>').replace('<html lang="zh-CN">','<html lang="zh-CN" data-view="tray">'));
  console.log('Fluent UI assets built; Iconify Fluent icons bundled locally.');
}
const ownPath=fileURLToPath(import.meta.url), invokedPath=process.argv[1]&&resolve(process.argv[1]);
if (process.platform==='win32'?ownPath.toLowerCase()===invokedPath?.toLowerCase():ownPath===invokedPath) await buildUI();
