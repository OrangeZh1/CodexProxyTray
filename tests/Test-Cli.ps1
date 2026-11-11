#requires -Version 7.2
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$BinaryPath,
    [string]$NodePath,
    [string]$TempRoot,
    [string]$EvidenceDirectory,
    [ValidateRange(1,60)][int]$ProcessTimeoutSeconds=15
)
$ErrorActionPreference='Stop'
if (-not $IsWindows) { throw 'This check requires the compiled Windows CLI and PowerShell 7 on Windows.' }
$sourceRoot=(Resolve-Path -LiteralPath (Split-Path -Parent $PSScriptRoot)).Path
$proxyNames=@('HTTP_PROXY','HTTPS_PROXY','ALL_PROXY','NO_PROXY','CODEX_PROXY_TRAY')
$parentNames=$proxyNames+@('LOCALAPPDATA','APPDATA','PATH','USERPROFILE','HOME','TEMP','TMP')
$nonce=[Guid]::NewGuid().ToString('N')
function Full-Path([string]$Value) {
    $provider=$null;$drive=$null
    $path=$ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Value,[ref]$provider,[ref]$drive)
    if ($provider.Name -ne 'FileSystem') { throw 'Choose a filesystem path.' }
    return [IO.Path]::GetFullPath($path)
}
function Is-Within([string]$Path,[string]$Directory) {
    return $Path.StartsWith($Directory.TrimEnd('\','/')+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)
}
function Check-NewTempDirectory([string]$Path,[switch]$AllowExisting) {
    $absolute=Full-Path $Path
    if ($absolute.Equals($sourceRoot,[StringComparison]::OrdinalIgnoreCase) -or (Is-Within $absolute $sourceRoot)) { throw 'Temporary output must stay outside the source checkout.' }
    $boundaries=@([IO.Path]::GetTempPath(),$env:RUNNER_TEMP) | Where-Object {$_} | ForEach-Object {Full-Path $_}
    $boundary=$boundaries | Where-Object {Is-Within $absolute $_} | Select-Object -First 1
    if (-not $boundary) { throw 'Use a new dedicated directory under RUNNER_TEMP or the OS temporary directory.' }
    if (-not $AllowExisting -and (Test-Path -LiteralPath $absolute)) { throw 'Choose a new temporary/evidence directory; existing files are preserved.' }
    $cursor=[IO.DirectoryInfo]::new($absolute)
    while ($cursor) {
        if ($cursor.Exists -and ($cursor.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Reparse points are not accepted in temporary output paths.' }
        if ($cursor.FullName.TrimEnd('\').Equals($boundary.TrimEnd('\'),[StringComparison]::OrdinalIgnoreCase)) { break }
        $cursor=$cursor.Parent
    }
    return $absolute
}
function Write-NewJson([string]$Path,$Value) {
    $stream=[IO.File]::Open($Path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    $writer=[IO.StreamWriter]::new($stream,[Text.UTF8Encoding]::new($false))
    try {$writer.Write(($Value|ConvertTo-Json -Depth 10))} finally {$writer.Dispose()}
}
function Fingerprint($Value) {
    $text=if($null -eq $Value){'unset'}else{'set:'+$Value}
    $hash=[Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($text))
    return [Convert]::ToHexString($hash).ToLowerInvariant()
}
function Environment-Fingerprints {
    $values=[ordered]@{}
    foreach($name in $parentNames){$values['Process/'+$name]=Fingerprint ([Environment]::GetEnvironmentVariable($name,'Process'))}
    foreach($scope in @('User','Machine')){foreach($name in $proxyNames+@('Path')){$values[$scope+'/'+$name]=Fingerprint ([Environment]::GetEnvironmentVariable($name,$scope))}}
    return $values
}
function Assert-Condition([bool]$Value,[string]$Message) {if(-not $Value){throw $Message}}
function Assert-Arguments($Actual,[string[]]$Expected) {
    $received=@($Actual)
    Assert-Condition ($received.Count -eq $Expected.Count) 'The mock CLI received an incorrect argument count.'
    for($index=0;$index -lt $Expected.Count;$index++){Assert-Condition ([string]::Equals([string]$received[$index],$Expected[$index],[StringComparison]::Ordinal)) ('An argument changed at index '+$index+'.')}
}
function Assert-ProxyValues($Actual,$Expected) {
    foreach($name in $proxyNames){
        $received=$Actual.$name;$wanted=$Expected[$name]
        if($null -eq $wanted){Assert-Condition ($null -eq $received) ('An unset proxy variable was changed: '+$name)}
        else {Assert-Condition ([string]::Equals([string]$received,[string]$wanted,[StringComparison]::Ordinal)) ('A proxy variable was changed incorrectly: '+$name)}
    }
}
function Assert-Target($Actual) {
    $target=@($Actual)
    Assert-Condition ($target.Count -eq 2) 'Proxy-info returned an unexpected target command.'
    for($index=0;$index -lt 2;$index++){
        $expected=@($nodeProgram,$mockScript)[$index]
        Assert-Condition ([string]::Equals([IO.Path]::GetFullPath([string]$target[$index]),[IO.Path]::GetFullPath($expected),[StringComparison]::OrdinalIgnoreCase)) 'Proxy-info did not resolve to the isolated Node/mock target.'
    }
}
function Stop-OwnedProcess([Diagnostics.Process]$Process) {
    if(-not $Process.HasExited){$Process.Kill($true);if(-not $Process.WaitForExit(5000)){throw 'The owned CLI check process tree could not be stopped.'}}
}
function Run-Process([string]$Program,[AllowEmptyCollection()][AllowEmptyString()][string[]]$ArgumentValues,$ProxyValues) {
    $start=[Diagnostics.ProcessStartInfo]::new();$start.FileName=$Program;$start.UseShellExecute=$false;$start.CreateNoWindow=$true;$start.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
    $start.WorkingDirectory=$qaTempRoot;$start.RedirectStandardOutput=$true;$start.RedirectStandardError=$true
    $start.StandardOutputEncoding=[Text.UTF8Encoding]::new($false);$start.StandardErrorEncoding=[Text.UTF8Encoding]::new($false)
    foreach($argument in $ArgumentValues){$start.ArgumentList.Add($argument)}
    $start.Environment.Clear()
    foreach($entry in $qaEnvironment.GetEnumerator()){$start.Environment[$entry.Key]=[string]$entry.Value}
    foreach($name in $proxyNames){if($null -ne $ProxyValues[$name]){$start.Environment[$name]=[string]$ProxyValues[$name]}}
    $process=[Diagnostics.Process]::new();$started=$false
    try {
        $process.StartInfo=$start;if(-not $process.Start()){throw 'Unable to start the owned CLI check process.'};$started=$true
        $stdout=$process.StandardOutput.ReadToEndAsync();$stderr=$process.StandardError.ReadToEndAsync()
        if(-not $process.WaitForExit($ProcessTimeoutSeconds*1000)){Stop-OwnedProcess $process;throw 'The owned CLI check process timed out.'}
        $output=$stdout.GetAwaiter().GetResult();$errorOutput=$stderr.GetAwaiter().GetResult()
        if(-not [string]::IsNullOrWhiteSpace($errorOutput)){
            # This child has only the fixed QA environment/mock inputs; keep a bounded diagnostic, never its full environment.
            $diagnostic=$errorOutput.Replace($qaTempRoot,'<QA_TEMP>').Replace($sourceRoot,'<SOURCE>').Replace($nodeProgram,'<NODE>')
            if($diagnostic.Length -gt 2000){$diagnostic=$diagnostic.Substring(0,2000)}
            throw ('The isolated CLI wrote stderr (exit '+$process.ExitCode+'): '+$diagnostic)
        }
        try {$document=ConvertFrom-Json -InputObject $output -ErrorAction Stop} catch {throw 'The CLI did not return the expected QA JSON; raw output is not retained.'}
        return [pscustomobject]@{exit_code=$process.ExitCode;document=$document}
    } finally {try {if($started){Stop-OwnedProcess $process}} finally {$process.Dispose()}}
}
function Run-Mock([string[]]$ArgumentValues,$ProxyValues,[int]$ExpectedExit,[switch]$Direct) {
    $result=if($Direct){Run-Process $nodeProgram (@($mockScript)+$ArgumentValues) $ProxyValues}else{Run-Process $cliBinary $ArgumentValues $ProxyValues}
    Assert-Condition ($result.exit_code -eq $ExpectedExit) 'The CLI did not propagate the expected mock exit code.'
    Assert-Condition ($result.document.fixture -eq 'CodexProxyTray CLI QA') 'The CLI did not invoke the fixed QA target.'
    Assert-Arguments $result.document.argv $ArgumentValues
    return $result.document
}
function Run-Case([string]$Name,[scriptblock]$Check) {
    $at=[Diagnostics.Stopwatch]::StartNew()
    try {$null=&$Check;$caseResults.Add([ordered]@{name=$Name;status='passed';milliseconds=$at.ElapsedMilliseconds})}
    catch {$caseResults.Add([ordered]@{name=$Name;status='failed';milliseconds=$at.ElapsedMilliseconds;error=$_.Exception.Message});throw}
}
function Remove-OwnedSandbox {
    [void](Check-NewTempDirectory $qaTempRoot -AllowExisting)
    $actual=(Get-Item -LiteralPath $qaTempRoot -Force).FullName
    Assert-Condition ([string]::Equals($actual,$qaTempRoot,[StringComparison]::OrdinalIgnoreCase)) 'Sandbox cleanup target changed; it was preserved.'
    Assert-Condition (-not ((Get-Item -LiteralPath $actual -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) 'Sandbox became a reparse point; it was preserved.'
    $owner=Get-Content -LiteralPath $ownerPath -Raw -Encoding utf8 | ConvertFrom-Json
    Assert-Condition ($owner.nonce -eq $nonce -and $owner.owner_pid -eq $PID) 'Sandbox ownership changed; it was preserved.'
    $queue=[Collections.Generic.Stack[IO.DirectoryInfo]]::new();$queue.Push([IO.DirectoryInfo]::new($actual))
    while($queue.Count){foreach($entry in $queue.Pop().EnumerateFileSystemInfos()){
        Assert-Condition (Is-Within $entry.FullName $actual) 'A cleanup target escaped the sandbox; it was preserved.'
        Assert-Condition (-not ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint)) 'A reparse point appeared in the sandbox; it was preserved.'
        Assert-Condition ($allowedPaths.Contains($entry.FullName)) 'An unknown file appeared in the sandbox; it was preserved.'
        if($entry -is [IO.DirectoryInfo]){$queue.Push($entry)}
    }}
    Remove-Item -LiteralPath $actual -Recurse -Force
    Assert-Condition (-not (Test-Path -LiteralPath $actual)) 'Sandbox cleanup did not complete.'
}

$cliBinary=(Resolve-Path -LiteralPath $BinaryPath).Path
if(-not (Test-Path -LiteralPath $cliBinary -PathType Leaf) -or [IO.Path]::GetFileName($cliBinary) -ne 'codex-proxy-cli.exe'){throw 'BinaryPath must explicitly name the compiled codex-proxy-cli.exe.'}
$nodeProgram=if($NodePath){(Resolve-Path -LiteralPath $NodePath).Path}else{(Get-Command node.exe -CommandType Application -ErrorAction Stop).Source}
if(-not (Test-Path -LiteralPath $nodeProgram -PathType Leaf) -or [IO.Path]::GetFileName($nodeProgram) -ne 'node.exe'){throw 'Use a real Node.js executable.'}
if(-not $TempRoot){$TempRoot=Join-Path ([IO.Path]::GetTempPath()) ('CodexProxyTray-cli-qa-'+$nonce)}
$qaTempRoot=Check-NewTempDirectory $TempRoot
if(-not $EvidenceDirectory){$EvidenceDirectory=$qaTempRoot+'-evidence'}
$evidenceRoot=Check-NewTempDirectory $EvidenceDirectory
if((Is-Within $evidenceRoot $qaTempRoot) -or (Is-Within $qaTempRoot $evidenceRoot) -or $evidenceRoot -eq $qaTempRoot){throw 'EvidenceDirectory must be separate from the disposable sandbox.'}
$baseline=Environment-Fingerprints
$caseResults=[Collections.Generic.List[object]]::new();$guard=$null;$guardOwned=$false;$sandboxCreated=$false;$sandboxOwned=$false;$failure=$null
$report=[ordered]@{format='codex-proxy-tray-cli-qa-v1';status='running';started_at=[DateTime]::UtcNow.ToString('o');binary_sha256=(Get-FileHash -LiteralPath $cliBinary -Algorithm SHA256).Hash.ToLowerInvariant();tests=$null;cases=$caseResults;scope='Isolated mock CLI only; no account, network, ChatGPT or registry changes';uncovered=@('interactive Ctrl+C','real Codex authentication/network','ChatGPT package activation/restart','GUI, tray and login autostart')}
[void][IO.Directory]::CreateDirectory($evidenceRoot)
Write-NewJson (Join-Path $evidenceRoot 'evidence-owner.json') ([ordered]@{nonce=$nonce;owner_pid=$PID;format=$report.format})
try {
    [void][IO.Directory]::CreateDirectory($qaTempRoot)
    $sandboxCreated=$true
    $ownerPath=Join-Path $qaTempRoot '.cli-test-owner.json';Write-NewJson $ownerPath ([ordered]@{nonce=$nonce;owner_pid=$PID});$sandboxOwned=$true
    $local=Join-Path $qaTempRoot 'local-app-data';$roaming=Join-Path $qaTempRoot 'roaming';$qaProfile=Join-Path $qaTempRoot 'profile';$scratch=Join-Path $qaTempRoot 'tmp'
    $runtime=Join-Path $local 'CodexProxyTray';$mockPackage=Join-Path $roaming 'npm/node_modules/@openai/codex';$mockBin=Join-Path $mockPackage 'bin';$mockScript=Join-Path $mockBin 'codex.js';$configPath=Join-Path $runtime 'config.json';$packagePath=Join-Path $mockPackage 'package.json'
    foreach($directory in @($runtime,$mockBin,$qaProfile,$scratch)){[void][IO.Directory]::CreateDirectory($directory)}
    [IO.File]::Copy((Join-Path $PSScriptRoot 'mock-codex.mjs'),$mockScript,$false)
    Write-NewJson $packagePath ([ordered]@{name='@openai/codex';version='0.0.0-qa';type='commonjs'})
    $proxyUrl='http://127.0.0.1:17890';Write-NewJson $configPath ([ordered]@{proxy_url=$proxyUrl;launch_desktop=$false;cli_command=@()})
    $allowedPaths=[Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach($path in @($ownerPath,$configPath,$packagePath,$mockScript,$qaProfile,$scratch)){
        $path=[IO.Path]::GetFullPath($path);[void]$allowedPaths.Add($path);$cursor=[IO.Path]::GetDirectoryName($path)
        while(Is-Within $cursor $qaTempRoot){[void]$allowedPaths.Add($cursor);$cursor=[IO.Path]::GetDirectoryName($cursor)}
    }
    $windows=[Environment]::GetFolderPath([Environment+SpecialFolder]::Windows)
    $qaEnvironment=[ordered]@{LOCALAPPDATA=$local;APPDATA=$roaming;USERPROFILE=$qaProfile;HOME=$qaProfile;TEMP=$scratch;TMP=$scratch;SystemRoot=$windows;WINDIR=$windows;ComSpec=(Join-Path $windows 'System32/cmd.exe');PATH=([IO.Path]::GetDirectoryName($nodeProgram)+';'+(Join-Path $windows 'System32'));PATHEXT='.COM;.EXE;.BAT;.CMD'}
    $passThrough=[ordered]@{HTTP_PROXY='http://parent.invalid:1081';HTTPS_PROXY='http://parent.invalid:1082';ALL_PROXY='socks5://parent.invalid:1083';NO_PROXY='parent.invalid,localhost';CODEX_PROXY_TRAY='qa-parent-marker'}
    $unset=[ordered]@{};foreach($name in $proxyNames){$unset[$name]=$null}
    $guarded=[ordered]@{HTTP_PROXY=$proxyUrl;HTTPS_PROXY=$proxyUrl;ALL_PROXY=$proxyUrl;NO_PROXY='localhost,127.0.0.1,::1';CODEX_PROXY_TRAY=('v1:'+$proxyUrl)}
    $complex=@('plain','with spaces','"quoted"',"apostrophe's",'trailing\','; echo not-a-shell','$(not-a-command)','a|b&c<d>e','你好 🌐','','a\"b')
    if(-not ('CodexProxyTray.CliQaEventName' -as [type])){Add-Type -TypeDefinition @'
using System.Text;
namespace CodexProxyTray {public static class CliQaEventName {public static string ForRoot(string root){ulong hash=14695981039346656037UL;unchecked{foreach(byte value in Encoding.UTF8.GetBytes(root.ToLowerInvariant())){hash^=value;hash*=1099511628211UL;}}return "Local\\CodexProxyTray.Guard."+hash.ToString("x16");}}}
'@}
    $eventName=[CodexProxyTray.CliQaEventName]::ForRoot($runtime)
    Run-Case 'inactive_proxy_info' {
        $result=Run-Process $cliBinary @('--proxy-info') $passThrough
        Assert-Condition ($result.exit_code -eq 0 -and $result.document.guard_active -eq $false -and $null -eq $result.document.proxy_url) 'Inactive proxy-info is incorrect.'
        Assert-Target $result.document.target
    }
    Run-Case 'inactive_passthrough_arguments_and_exit' {$argsForMock=$complex+@('--mock-exit=19');$document=Run-Mock $argsForMock $passThrough 19;Assert-ProxyValues $document.proxy $passThrough}
    Run-Case 'inactive_unset_environment' {$document=Run-Mock @('--mock-exit=0') $unset 0;Assert-ProxyValues $document.proxy $unset}
    $created=$false;$guard=[Threading.EventWaitHandle]::new($false,[Threading.EventResetMode]::ManualReset,$eventName,[ref]$created)
    if(-not $created){$guard.Dispose();$guard=$null;throw 'The QA event already existed; it was not signaled.'};$guardOwned=$true
    Run-Case 'unsignaled_event_is_inactive' {$result=Run-Process $cliBinary @('--proxy-info') $unset;Assert-Condition ($result.exit_code -eq 0 -and $result.document.guard_active -eq $false) 'An unsignaled event was treated as active.'}
    [void]$guard.Set()
    Run-Case 'active_proxy_info' {$result=Run-Process $cliBinary @('--proxy-info') $unset;Assert-Condition ($result.exit_code -eq 0 -and $result.document.guard_active -eq $true -and $result.document.proxy_url -eq $proxyUrl) 'Active proxy-info is incorrect.';Assert-Target $result.document.target}
    Run-Case 'active_scoped_injection_arguments_and_exit' {$argsForMock=$complex+@('--mock-exit=37');$document=Run-Mock $argsForMock $passThrough 37;Assert-ProxyValues $document.proxy $guarded}
    Run-Case 'active_injection_from_unset_environment' {$document=Run-Mock @('--mock-exit=0') $unset 0;Assert-ProxyValues $document.proxy $guarded}
    Run-Case 'ordinary_node_child_is_unchanged' {$document=Run-Mock @('--mock-exit=0') $passThrough 0 -Direct;Assert-ProxyValues $document.proxy $passThrough}
    [void]$guard.Reset()
    Run-Case 'reset_event_restores_passthrough' {$document=Run-Mock @('--mock-exit=0') $passThrough 0;Assert-ProxyValues $document.proxy $passThrough}
    $report.status='passed'
} catch {$failure=$_;$report.status='failed';$report.error=$_.Exception.Message}
finally {
    if($guard){try {if($guardOwned){[void]$guard.Reset()}} catch {$report.status='failed';$report.guard_cleanup_error=$_.Exception.Message;if(-not $failure){$failure=$_}} finally {$guard.Dispose()}}
    try {
        $after=Environment-Fingerprints
        foreach($key in $baseline.Keys){Assert-Condition ($baseline[$key] -eq $after[$key]) ('The parent/User/Machine environment changed: '+$key)}
        $report.environment_unchanged=$true
    } catch {$report.status='failed';$report.environment_unchanged=$false;$report.environment_error=$_.Exception.Message;if(-not $failure){$failure=$_}}
    if($sandboxOwned){try {Remove-OwnedSandbox;$report.sandbox_cleanup='passed'} catch {$report.status='failed';$report.sandbox_cleanup='blocked';$report.cleanup_error=$_.Exception.Message;$report.retained_sandbox=$qaTempRoot;if(-not $failure){$failure=$_}}}
    elseif($sandboxCreated){$report.sandbox_cleanup='blocked';$report.retained_sandbox=$qaTempRoot}
    $report.tests=[ordered]@{pass=@($caseResults|Where-Object status -eq 'passed').Count;fail=@($caseResults|Where-Object status -eq 'failed').Count}
    $report.finished_at=[DateTime]::UtcNow.ToString('o')
    [void](Check-NewTempDirectory $evidenceRoot -AllowExisting)
    Write-NewJson (Join-Path $evidenceRoot 'cli-checks.json') $report
}
if($failure){throw $failure}
Write-Output ('Isolated Windows CLI checks passed: '+$caseResults.Count+' cases; evidence: '+(Join-Path $evidenceRoot 'cli-checks.json'))
