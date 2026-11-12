param([string]$EvidenceDirectory, [string]$BaselinePath, [switch]$CaptureBaseline)
$ErrorActionPreference = 'Stop'
$proxyNames = @('HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'NO_PROXY')
if (-not ('CodexProxyTray.NativeCheckIdentity' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
namespace CodexProxyTray {
    public sealed class CheckIdentity {
        public int package_status;
        public string package_family;
        public bool in_job;
    }
    public static class NativeCheckIdentity {
        [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr OpenProcess(uint access,bool inherit,int pid);
        [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr process);
        [DllImport("kernel32.dll",CharSet=CharSet.Unicode)] static extern int GetPackageFamilyName(IntPtr process,ref uint length,StringBuilder name);
        [DllImport("kernel32.dll",SetLastError=true)] static extern bool IsProcessInJob(IntPtr process,IntPtr job,out bool inside);
        public static CheckIdentity Read(int pid) {
            IntPtr process=OpenProcess(0x1000,false,pid);if(process==IntPtr.Zero)throw new Win32Exception();
            try {uint length=512;var family=new StringBuilder((int)length);int code=GetPackageFamilyName(process,ref length,family);if(code!=0&&code!=15700)throw new Win32Exception(code);bool job;if(!IsProcessInJob(process,IntPtr.Zero,out job))throw new Win32Exception();return new CheckIdentity{package_status=code,package_family=family.ToString(),in_job=job};}
            finally {CloseHandle(process);}
        }
    }
}
'@
}
$harnessIdentity=[CodexProxyTray.NativeCheckIdentity]::Read($PID)
if ($harnessIdentity.package_status -ne 15700) { throw 'This check is running in an MSIX package context and may read virtualized AppData. Run it through tests/Start-Independent.ps1 -CheckScript instead.' }
function Write-NewJson([string]$Path, $Value, [int]$Depth=8) {
    $stream=[IO.File]::Open($Path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    $writer=[IO.StreamWriter]::new($stream,[Text.UTF8Encoding]::new($false))
    try { $writer.Write(($Value | ConvertTo-Json -Depth $Depth)) } finally { $writer.Dispose() }
}
function Write-NewText([string]$Path,[string]$Text) {
    $stream=[IO.File]::Open($Path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    $writer=[IO.StreamWriter]::new($stream,[Text.UTF8Encoding]::new($false))
    try {$writer.Write($Text)} finally {$writer.Dispose()}
}
function Fingerprint($Value) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $text = if ($null -eq $Value) { 'unset' } else { 'set:' + $Value }
        return ([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($text)))).Replace('-', '').ToLowerInvariant()
    } finally { $sha.Dispose() }
}
function Proxy-Fingerprints {
    $result = [ordered]@{}
    foreach ($scope in @('User', 'Machine')) { foreach ($name in $proxyNames) { $result["$scope/$name"] = Fingerprint ([Environment]::GetEnvironmentVariable($name, $scope)) } }
    return $result
}
function Assert-Fingerprints($Expected, $Actual) {
    foreach ($key in $Actual.Keys) { if (-not $Expected.Contains($key) -or $Expected[$key] -ne $Actual[$key]) { throw "Global proxy environment changed: $key" } }
}
function Literal([string]$Value) { return "'" + $Value.Replace("'", "''") + "'" }
function Encoded([string]$Code) { return [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($Code)) }
function Stop-CheckProcess([Diagnostics.Process]$Process) {
    if ($Process.HasExited) { return }
    if ($Process.GetType().GetMethod('Kill', [type[]]@([bool]))) { $Process.Kill($true) }
    else {
        # Windows PowerShell 5.1 lacks Kill(entireProcessTree). This PID belongs to the check we just started.
        $kill = New-Object Diagnostics.ProcessStartInfo
        $kill.FileName = Join-Path $env:SystemRoot 'System32\taskkill.exe'
        $kill.Arguments = '/PID ' + $Process.Id + ' /T /F'
        $kill.UseShellExecute = $false; $kill.CreateNoWindow = $true; $kill.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
        $killer = [Diagnostics.Process]::Start($kill)
        try {
            if (-not $killer.WaitForExit(10000)) { $killer.Kill(); throw 'The owned check process tree could not be stopped within the timeout.' }
            if ($killer.ExitCode -ne 0 -and -not $Process.HasExited) { throw 'The owned check process tree could not be stopped.' }
        } finally { $killer.Dispose() }
    }
    if (-not $Process.WaitForExit(5000)) { throw 'The owned check process has not exited after termination.' }
}
function Run-HiddenShell([string]$Arguments, [string]$LogPrefix) {
    $start=New-Object Diagnostics.ProcessStartInfo
    $start.FileName=$shell; $start.Arguments=$Arguments; $start.UseShellExecute=$false; $start.CreateNoWindow=$true; $start.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
    $start.RedirectStandardOutput=$true; $start.RedirectStandardError=$true
    $process=New-Object Diagnostics.Process
    try {
        $process.StartInfo=$start
        if(-not $process.Start()){throw 'Unable to start the owned check launcher.'}
        # Keep this Process/handle from Start through ExitCode; Start-Process on PS 5.1 can lose the exit code.
        $stdout=$process.StandardOutput.ReadToEndAsync();$stderr=$process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(10000)) { Stop-CheckProcess $process; throw 'The check launcher timed out.' }
        Write-NewText ($LogPrefix+'.stdout.log') ($stdout.GetAwaiter().GetResult())
        Write-NewText ($LogPrefix+'.stderr.log') ($stderr.GetAwaiter().GetResult())
        if ($process.ExitCode -ne 0) { throw ('Check launcher exit '+$process.ExitCode+'; see ' + $LogPrefix + '.stderr.log') }
    } finally {
        if (-not $process.HasExited) { Stop-CheckProcess $process }
        $process.Dispose()
    }
}
if ($CaptureBaseline) {
    if (-not $BaselinePath) { throw '-BaselinePath is required with -CaptureBaseline.' }
    $BaselinePath=$ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($BaselinePath)
    if (Test-Path -LiteralPath $BaselinePath) { throw 'Choose a new baseline file; existing evidence is preserved.' }
    Write-NewJson $BaselinePath ([ordered]@{ captured_at = [DateTime]::UtcNow.ToString('o'); harness_identity=$harnessIdentity; fingerprints = (Proxy-Fingerprints) })
    Write-Output ('Proxy environment baseline saved: ' + $BaselinePath)
    return
}
if (-not $EvidenceDirectory) { throw '-EvidenceDirectory is required for native checks.' }
if (Test-Path -LiteralPath $EvidenceDirectory) {
    if (@(Get-ChildItem -LiteralPath $EvidenceDirectory -Force).Count) { throw 'Choose a new, empty evidence directory; existing runs are preserved.' }
} else { New-Item -ItemType Directory -Path $EvidenceDirectory | Out-Null }
$EvidenceDirectory = (Resolve-Path -LiteralPath $EvidenceDirectory).Path
Write-NewJson (Join-Path $EvidenceDirectory 'evidence-owner.json') ([ordered]@{format='codex-proxy-tray-native-v1';owner_pid=$PID;started_at=[DateTime]::UtcNow.ToString('o');harness_identity=$harnessIdentity})
$runtime = Join-Path $env:LOCALAPPDATA 'CodexProxyTray'
$shell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$fixture = Join-Path $PSScriptRoot 'Dump-PackageEnvironment.ps1'
function Helper-Status {
    $value = Get-Content -LiteralPath (Join-Path $runtime 'status.json') -Raw -Encoding utf8 | ConvertFrom-Json
    $age = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() - [long]$value.updated_at
    if (-not $value.backend_ready -or -not $value.guard_ready -or $age -lt 0 -or $age -gt 10000) { throw 'Start the desktop helper and enable its guard before this check.' }
    $helper = Get-Process -Id $value.helper_pid -ErrorAction Stop
    if ($helper.ProcessName -ne 'CodexProxyTray') { throw 'The status receipt does not identify a live CodexProxyTray process.' }
    $identity=[CodexProxyTray.NativeCheckIdentity]::Read($helper.Id)
    if ($identity.package_status -ne 15700 -or $identity.in_job) { throw 'The helper must be unpackaged and outside a job before independent lifecycle checks.' }
    return $value
}
$report = [ordered]@{ status = 'running'; started_at = [DateTime]::UtcNow.ToString('o'); harness_identity=$harnessIdentity; package_activations = @(); startup_global_proxy_environment_checked = [bool]$BaselinePath; desktop_restart_performed = $false }
try {
    $expectedGlobals = Proxy-Fingerprints
    if ($BaselinePath) {
        $baseline = Get-Content -LiteralPath $BaselinePath -Raw -Encoding utf8 | ConvertFrom-Json
        $expectedGlobals = [ordered]@{}
        foreach ($property in $baseline.fingerprints.PSObject.Properties) { $expectedGlobals[$property.Name] = [string]$property.Value }
        Assert-Fingerprints $expectedGlobals (Proxy-Fingerprints)
    }
    if ([Environment]::GetEnvironmentVariable('CODEX_PROXY_TRAY', 'Process')) { throw 'Run this check from an ordinary shell, not from a guarded CLI process.' }
    $parentProcess = @{}
    foreach ($name in $proxyNames + 'CODEX_PROXY_TRAY') { $parentProcess[$name] = Fingerprint ([Environment]::GetEnvironmentVariable($name, 'Process')) }
    $status = Helper-Status
    $report.helper_identity=[CodexProxyTray.NativeCheckIdentity]::Read([int]$status.helper_pid)
    $helperProcess=Get-Process -Id $status.helper_pid -ErrorAction Stop
    $report.helper_executable=[ordered]@{pid=$helperProcess.Id;path=$helperProcess.Path;sha256=(Get-FileHash -LiteralPath $helperProcess.Path -Algorithm SHA256).Hash.ToLowerInvariant()}
    $pids = @()
    for ($i = 1; $i -le 2; $i++) {
        $output = Join-Path $EvidenceDirectory "package-child-$i.json"
        $childArgs = '-NoLogo -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "{0}" -OutputPath "{1}"' -f $fixture, $output
        $invocation = '$ErrorActionPreference=''Stop''; Invoke-CommandInDesktopPackage -PackageFamilyName OpenAI.Codex_2p2nqsd0c76g0 -AppId App -Command {0} -Args {1} -PreventBreakaway' -f (Literal $shell), (Literal $childArgs)
        Run-HiddenShell ('-NoLogo -NoProfile -NonInteractive -EncodedCommand ' + (Encoded $invocation)) (Join-Path $EvidenceDirectory "package-launcher-$i")
        $deadline = [DateTime]::UtcNow.AddSeconds(10)
        while (-not (Test-Path -LiteralPath $output) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 150 }
        $result = Get-Content -LiteralPath $output -Raw -Encoding utf8 | ConvertFrom-Json
        if ($result.package_status -ne 0 -or $result.package_family -ne 'OpenAI.Codex_2p2nqsd0c76g0') { throw "Package activation $i did not run inside the expected application package." }
        foreach ($name in @('HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY')) { if ($result.fingerprints.$name -ne (Fingerprint $status.proxy_url)) { throw "Package activation $i has an incorrect $name environment." } }
        if ($result.fingerprints.NO_PROXY -ne (Fingerprint 'localhost,127.0.0.1,::1') -or $result.fingerprints.CODEX_PROXY_TRAY -ne (Fingerprint ('v1:' + $status.proxy_url))) { throw "Package activation $i has an incorrect bypass or guard marker." }
        if ([int]$result.pid -le 0 -or $pids -contains [int]$result.pid) { throw 'Package activations did not identify distinct new processes.' }
        $pids += [int]$result.pid
        $report.package_activations += [ordered]@{ pid = [int]$result.pid; package_family=$result.package_family; package_identity_verified=$true; proxy_environment_verified = $true }
    }
    $ordinary = Join-Path $EvidenceDirectory 'ordinary-child.json'
    $ordinaryArgs = '-NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "{0}" -OutputPath "{1}"' -f $fixture, $ordinary
    Run-HiddenShell $ordinaryArgs (Join-Path $EvidenceDirectory 'ordinary-launcher')
    $ordinaryResult = Get-Content -LiteralPath $ordinary -Raw -Encoding utf8 | ConvertFrom-Json
    if ($ordinaryResult.package_status -ne 15700) { throw 'The ordinary process unexpectedly has an application package identity.' }
    foreach ($name in $parentProcess.Keys) { if ($ordinaryResult.fingerprints.$name -ne $parentProcess[$name]) { throw "Ordinary process environment changed: $name" } }
    $report.ordinary_process_environment_unchanged = $true
    # Rebuild the user environment from Windows; this shell may have a cached PATH.
    if (-not ('CodexProxyTray.NativeCheckEnvironment' -as [type])) {
        Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Security.Principal;
namespace CodexProxyTray {
    public static class NativeCheckEnvironment {
        [DllImport("userenv.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool CreateEnvironmentBlock(out IntPtr block, IntPtr token, [MarshalAs(UnmanagedType.Bool)] bool inherit);
        [DllImport("userenv.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool DestroyEnvironmentBlock(IntPtr block);
        public static Dictionary<string, string> Read() {
            using (var identity = WindowsIdentity.GetCurrent()) {
                IntPtr block;
                if (!CreateEnvironmentBlock(out block, identity.Token, false)) throw new Win32Exception(Marshal.GetLastWin32Error());
                try {
                    var result = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
                    var cursor = block;
                    while (true) {
                        var entry = Marshal.PtrToStringUni(cursor);
                        if (String.IsNullOrEmpty(entry)) return result;
                        var separator = entry.IndexOf('=');
                        if (separator > 0) result[entry.Substring(0, separator)] = entry.Substring(separator + 1);
                        cursor = IntPtr.Add(cursor, checked((entry.Length + 1) * 2));
                    }
                } finally { DestroyEnvironmentBlock(block); }
            }
        }
    }
}
'@
    }
    $freshCode = @'
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$command=Get-Command codex -ErrorAction Stop | Select-Object -First 1
$info=& $env:CODEX_PROXY_CHECK_SHIM --proxy-info | ConvertFrom-Json
if($LASTEXITCODE -ne 0){throw 'CLI proxy-info failed'}
$version=(& $env:CODEX_PROXY_CHECK_SHIM --version | Out-String).Trim()
if($LASTEXITCODE -ne 0 -or -not $version){throw 'CLI version failed'}
[ordered]@{resolved_path=$command.Source;command_type=[string]$command.CommandType;guard_active=[bool]$info.guard_active;proxy_url=$info.proxy_url;target=$info.target;cli_version=$version}|ConvertTo-Json -Compress
'@
    $fresh = [CodexProxyTray.NativeCheckEnvironment]::Read()
    $start = New-Object Diagnostics.ProcessStartInfo
    $start.FileName = $shell
    $start.Arguments = '-NoLogo -NoProfile -NonInteractive -EncodedCommand ' + (Encoded $freshCode)
    $start.UseShellExecute = $false; $start.CreateNoWindow = $true; $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $start.RedirectStandardOutput = $true; $start.RedirectStandardError = $true
    $start.StandardOutputEncoding=[Text.UTF8Encoding]::new($false)
    $start.StandardErrorEncoding=[Text.UTF8Encoding]::new($false)
    $start.EnvironmentVariables.Clear()
    foreach ($key in $fresh.Keys) { $start.EnvironmentVariables[$key] = $fresh[$key] }
    $start.EnvironmentVariables['CODEX_PROXY_CHECK_SHIM']=Join-Path $runtime 'cli\codex-proxy-cli.exe'
    $child = New-Object Diagnostics.Process
    $childStarted=$false
    try {
        $child.StartInfo = $start
        if (-not $child.Start()) { throw 'Unable to start the fresh-environment check.' }
        $childStarted=$true
        $stdout = $child.StandardOutput.ReadToEndAsync(); $stderr = $child.StandardError.ReadToEndAsync()
        if (-not $child.WaitForExit(20000)) { Stop-CheckProcess $child; throw 'Fresh-environment check timed out.' }
        $output = $stdout.GetAwaiter().GetResult(); $errorOutput = $stderr.GetAwaiter().GetResult()
        if ($child.ExitCode -ne 0) { throw ('Fresh-environment CLI check failed: ' + $errorOutput) }
        $cli = $output | ConvertFrom-Json
    } finally { try {if ($childStarted -and -not $child.HasExited) { Stop-CheckProcess $child }} finally {$child.Dispose()} }
    $expectedShim=[IO.Path]::GetFullPath((Join-Path $runtime 'cli\codex-proxy-cli.exe'))
    $allowed=@([string]$cli.target[0])
    if(@($cli.target).Count -eq 2 -and [IO.Path]::GetFileName([string]$cli.target[0]) -ieq 'node.exe'){$prefix=[string]$cli.target[1];for($index=0;$index -lt 5;$index++){$prefix=[IO.Path]::GetDirectoryName($prefix)};$allowed=@('codex.cmd','codex.ps1','codex.exe','codex'|ForEach-Object {Join-Path $prefix $_})}
    if($cli.command_type -notin @('Application','ExternalScript') -or -not @($allowed|Where-Object {[string]::Equals([IO.Path]::GetFullPath($_),[IO.Path]::GetFullPath($cli.resolved_path),[StringComparison]::OrdinalIgnoreCase)}).Count){throw 'The user codex command does not resolve to the original installation.'}
    if([IO.Path]::GetFullPath($cli.resolved_path).StartsWith($runtime+'\',[StringComparison]::OrdinalIgnoreCase)){throw 'The internal proxy entry must not shadow the official user codex command.'}
    if (-not $cli.guard_active -or $cli.proxy_url -ne $status.proxy_url) { throw 'The CLI resolved from the new user environment is not guarded.' }
    $report.fresh_user_environment_cli_resolution = [ordered]@{ resolved_path = $cli.resolved_path; guard_active = $cli.guard_active; cli_version = $cli.cli_version }
    $after = Helper-Status
    if ($after.helper_pid -ne $status.helper_pid -or $after.proxy_url -ne $status.proxy_url) { throw 'The helper or its proxy configuration changed during the check.' }
    Assert-Fingerprints $expectedGlobals (Proxy-Fingerprints)
    $report.global_proxy_environment_unchanged_during_checks = $true
    $report.status = 'passed'
} catch {
    $report.status = 'failed'; $report.error = $_.Exception.Message
    throw
} finally {
    $report.finished_at = [DateTime]::UtcNow.ToString('o')
    Write-NewJson (Join-Path $EvidenceDirectory 'native-checks.json') $report
}
Write-Output ('Native package and fresh-environment CLI checks passed: ' + $EvidenceDirectory)
Write-Output 'ChatGPT main-process restart has not been performed.'
