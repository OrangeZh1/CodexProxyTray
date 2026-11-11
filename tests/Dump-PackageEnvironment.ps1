param([Parameter(Mandatory=$true)][string]$OutputPath)
$ErrorActionPreference = 'Stop'
if (-not ('CodexProxyTray.ProbeIdentity' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
namespace CodexProxyTray {
    public static class ProbeIdentity {
        [DllImport("kernel32.dll")] static extern IntPtr GetCurrentProcess();
        [DllImport("kernel32.dll",CharSet=CharSet.Unicode)] static extern int GetPackageFamilyName(IntPtr process,ref uint length,StringBuilder name);
        public static string[] Read() {uint length=512;var name=new StringBuilder((int)length);int status=GetPackageFamilyName(GetCurrentProcess(),ref length,name);return new string[]{status.ToString(),name.ToString()};}
    }
}
'@
}
$identity=[CodexProxyTray.ProbeIdentity]::Read()
$fingerprints = [ordered]@{}
foreach ($name in @('HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'NO_PROXY', 'CODEX_PROXY_TRAY')) {
    $value = [Environment]::GetEnvironmentVariable($name, 'Process')
    $text = if ($null -eq $value) { 'unset' } else { 'set:' + $value }
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $fingerprints[$name] = ([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($text)))).Replace('-', '').ToLowerInvariant() }
    finally { $sha.Dispose() }
}
$json = [ordered]@{ pid = $PID; package_status=[int]$identity[0]; package_family=$identity[1]; fingerprints = $fingerprints } | ConvertTo-Json -Depth 4
$temporary = $OutputPath + '.tmp'
$stream=[IO.File]::Open($temporary,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
$writer=[IO.StreamWriter]::new($stream,[Text.UTF8Encoding]::new($false))
try {$writer.Write($json)} finally {$writer.Dispose()}
[IO.File]::Move($temporary, $OutputPath)
