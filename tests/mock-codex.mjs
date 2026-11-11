// Fixed QA target: also valid as CommonJS when copied to npm's codex.js path.
// It never reads account files, makes network requests, or prints the full environment.
const argv = process.argv.slice(2);
const names = ['HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'NO_PROXY', 'CODEX_PROXY_TRAY'];
const requested = argv.find(argument => argument.startsWith('--mock-exit='));
const exitCode = requested ? Number(requested.slice('--mock-exit='.length)) : 0;
if (!Number.isInteger(exitCode) || exitCode < 0 || exitCode > 125) {
  process.stderr.write('Invalid fixed QA exit code.\n');
  process.exitCode = 2;
} else {
  process.stdout.write(JSON.stringify({
    fixture: 'CodexProxyTray CLI QA',
    argv,
    proxy: Object.fromEntries(names.map(name => [name, process.env[name] ?? null])),
  }) + '\n');
  process.exitCode = exitCode;
}
