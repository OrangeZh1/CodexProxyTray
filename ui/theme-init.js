// Synchronous, local-only first paint. Keep the preference contract in themes.mjs.
(() => {
  let mode='system';
  try {const saved=localStorage.getItem('color-theme');if(['system','light','dark'].includes(saved))mode=saved;}catch{}
  let systemDark=false;
  try {systemDark=!!window.matchMedia?.('(prefers-color-scheme: dark)').matches;}catch{}
  const resolved=mode==='dark'||mode==='system'&&systemDark?'dark':'light';
  document.documentElement.dataset.theme=resolved;
  document.documentElement.style.colorScheme=resolved;
})();
