param([Parameter(Mandatory=$true)][string]$ReleaseDirectory,[Parameter(Mandatory=$true)][string]$EvidenceDirectory)
$ErrorActionPreference='Stop'
if($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted'){throw 'Updater acceptance runs only in isolated GitHub-hosted Windows.'}
$release=(Resolve-Path -LiteralPath $ReleaseDirectory).Path
if(Test-Path -LiteralPath $EvidenceDirectory){throw 'Existing evidence is retained.'}
[void](New-Item -ItemType Directory -Path $EvidenceDirectory)
$root=Join-Path $env:RUNNER_TEMP ('CodexProxyTray-updater-'+[Guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $root)
$manifest=Get-Content -LiteralPath (Join-Path $release 'CodexProxyTray-update.json') -Raw|ConvertFrom-Json
$zip=Join-Path $release ("CodexProxyTray-v$($manifest.version)-windows-x64.zip")
$report=[ordered]@{schema=1;status='running';checks=@();scope='Isolated real helper restart, hash/path rejection, rollback and config/log preservation; no real user profiles, network or service changes';cleanup='pending'}
function Hash([string]$Path){(Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()}
function WriteJson([string]$Path,$Value){[IO.File]::WriteAllText($Path,($Value|ConvertTo-Json -Depth 20),[Text.UTF8Encoding]::new($false))}
function Require([bool]$Condition,[string]$Name){if(-not $Condition){throw $Name};$script:report.checks+=@{name=$Name;status='passed'}}
function StartOwned($Case,[string]$File,[string[]]$Arguments){$info=[Diagnostics.ProcessStartInfo]::new($File);$info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.WindowStyle='Hidden';$info.WorkingDirectory=[IO.Path]::GetDirectoryName($File);$info.Environment['LOCALAPPDATA']=$Case.profile;$info.Environment['APPDATA']=Join-Path $Case.profile 'Roaming';$info.Environment['WEBVIEW2_USER_DATA_FOLDER']=Join-Path $Case.profile 'webview';foreach($argument in $Arguments){$info.ArgumentList.Add($argument)};[Diagnostics.Process]::Start($info)}
function WaitFor([scriptblock]$Check,[int]$Seconds,[string]$Name){$end=[DateTime]::UtcNow.AddSeconds($Seconds);do{if(& $Check){return};Start-Sleep -Milliseconds 200}while([DateTime]::UtcNow -lt $end);throw $Name}
function Case([string]$Name){
    $directory=Join-Path $root $Name;$profile=Join-Path $directory 'profile';$data=Join-Path $profile 'CodexProxyTray';$target=Join-Path $directory 'app\current'
    foreach($path in @($data,(Join-Path $data 'logs'),(Join-Path $data 'updates'),$target)){[void](New-Item -ItemType Directory -Path $path -Force)}
    [IO.Compression.ZipFile]::ExtractToDirectory($zip,$target)
    WriteJson (Join-Path $data 'config.json') @{launch_target='none';network_enabled=$false;auto_check_updates=$false;connectivity_enabled=$false;traffic_enabled=$false;proxy_url='http://127.0.0.1:17890'}
    WriteJson (Join-Path $data 'logs\events-v1.json') @{schema=1;records=@(@{id=1;at=1;level='info';category='fixture';message='preserve-this-record';details='synthetic local data'})}
    $files=@(Get-ChildItem -LiteralPath $target|ForEach-Object {@{name=$_.Name;sha256=Hash $_.FullName}})
    WriteJson (Join-Path $directory 'app\active-version.json') @{schema=1;state='active';current=@{path=$target;files=$files;source_commit=$env:GITHUB_SHA};recovery_references=@()}
    WriteJson (Join-Path $directory 'app\install-journal.json') @{schema=1;state='installed'}
    $case=[pscustomobject]@{directory=$directory;profile=$profile;data=$data;target=$target;exe=(Join-Path $target 'CodexProxyTray.exe');parent=$null;job=$null;installer=$null}
    $case.parent=StartOwned $case $case.exe @('--background')
    WaitFor {try{$value=Get-Content -LiteralPath (Join-Path $data 'ui-ready-main.json') -Raw|ConvertFrom-Json;$value.helper_pid -eq $case.parent.Id -and $value.rendered}catch{$false}} 45 'Initial helper did not render'
    return $case
}
function Job($Case,[string]$Archive){$path=Join-Path $Case.data ('updates\'+$Case.parent.Id+'-'+[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds());[void](New-Item -ItemType Directory -Path $path);Copy-Item -LiteralPath $Archive -Destination (Join-Path $path 'package.zip');Copy-Item -LiteralPath (Join-Path $release 'CodexProxyTray-update.json') -Destination (Join-Path $path 'manifest.json');[IO.File]::WriteAllText((Join-Path $path 'installer.ps1'),[IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\src-tauri\src\install_update.ps1')),[Text.UTF8Encoding]::new($true));WriteJson (Join-Path $path 'plan.json') @{schema=1;product='CodexProxyTray';version=$manifest.version;zip_sha256=Hash (Join-Path $path 'package.zip');manifest_sha256=Hash (Join-Path $path 'manifest.json');executable=$Case.exe;data_root=$Case.data;parent_pid=$Case.parent.Id;parent_created=$Case.parent.StartTime.ToUniversalTime().ToFileTimeUtc().ToString()};$Case.job=$path;return $path}
function Installer($Case,[string]$Phase){$shell=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe';StartOwned $Case $shell @('-NoLogo','-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',(Join-Path $Case.job 'installer.ps1'),'-Phase',$Phase,'-Job',$Case.job)}
function Quit($Case){$quitter=StartOwned $Case $Case.exe @('--quit');[void]$quitter.WaitForExit(10000);WaitFor {$Case.parent.HasExited} 20 'Owned helper refused graceful quit'}
function StopCase($Case){foreach($process in @(Get-CimInstance Win32_Process -Filter "name='CodexProxyTray.exe'"|Where-Object {$_.ExecutablePath -and $_.ExecutablePath.StartsWith($Case.directory+'\',[StringComparison]::OrdinalIgnoreCase)})){$request=StartOwned $Case $process.ExecutablePath @('--quit');[void]$request.WaitForExit(10000);$owned=Get-Process -Id $process.ProcessId -ErrorAction SilentlyContinue;if($owned -and -not $owned.WaitForExit(10000)){throw 'Owned fixture did not stop; diagnostic tree retained'}}}
$cases=@()
try {
    if(@(Get-Process -Name CodexProxyTray -ErrorAction SilentlyContinue).Count){throw 'Another helper exists; it was not touched.'}
    $good=Case 'success';$cases+=$good;$job=Job $good $zip
    $planPath=Join-Path $job 'plan.json';$plan=Get-Content -LiteralPath $planPath -Raw|ConvertFrom-Json;$validHash=$plan.zip_sha256;$plan.zip_sha256='0'*64;WriteJson $planPath $plan
    $check=Installer $good 'Prepare';Require ($check.WaitForExit(45000) -and $check.ExitCode -ne 0 -and -not $good.parent.HasExited) 'corrupt_archive_rejected_before_quit'
    $unknown=Join-Path $good.target 'user-notes.txt';[IO.File]::WriteAllText($unknown,'owned synthetic probe');$job=Job $good $zip;$check=Installer $good 'Prepare';Require ($check.WaitForExit(45000) -and $check.ExitCode -ne 0 -and (Test-Path -LiteralPath $unknown) -and -not $good.parent.HasExited) 'unknown_install_file_is_retained_and_blocks_update';Remove-Item -LiteralPath $unknown
    # Use a fresh task ID; previous failed evidence is retained.
    $job=Job $good $zip;$check=Installer $good 'Prepare';Require ($check.WaitForExit(45000) -and $check.ExitCode -eq 0) 'valid_official_package_and_layout_accepted'
    $installer=Installer $good 'Apply';$good.installer=$installer;WaitFor {Test-Path -LiteralPath (Join-Path $job 'handoff-ready.json')} 10 'Installer handoff failed'
    $configHash=Hash (Join-Path $good.data 'config.json');Quit $good
    Require ($installer.WaitForExit(60000) -and $installer.ExitCode -eq 0) 'real_update_restarted_and_passed_both_views'
    $result=Get-Content -LiteralPath (Join-Path $job 'result.json') -Raw|ConvertFrom-Json;Require ($result.phase -eq 'installed') 'success_receipt_committed'
    Require ((Hash (Join-Path $job 'config.json')) -eq $configHash) 'original_configuration_archived_exactly'
    $logs=Get-Content -LiteralPath (Join-Path $job 'events-v1.json') -Raw|ConvertFrom-Json;Require (@($logs.records|Where-Object message -eq 'preserve-this-record').Count -eq 1) 'all_pre_update_log_records_archived'
    Require ((Test-Path -LiteralPath (Join-Path $good.directory 'app\previous\CodexProxyTray.exe')) -and -not (Test-Path -LiteralPath (Join-Path $job 'package.zip'))) 'one_previous_version_and_download_cleanup'
    Copy-Item -LiteralPath (Join-Path $job 'result.json') -Destination (Join-Path $EvidenceDirectory 'success-result.json');StopCase $good
    # A validly hashed synthetic package whose executable exits without readiness
    # exercises the actual rollback; no production-only test bypass is added.
    $bad=Case 'rollback';$cases+=$bad;$fake=Join-Path $bad.directory 'fake';[void](New-Item -ItemType Directory -Path $fake);[IO.Compression.ZipFile]::ExtractToDirectory($zip,$fake)
    $code=Join-Path $bad.directory 'NoReadiness.cs';[IO.File]::WriteAllText($code,('[assembly:System.Reflection.AssemblyFileVersion("'+$manifest.version+'.0")][assembly:System.Reflection.AssemblyInformationalVersion("'+$manifest.version+'")]class Program{static void Main(){System.IO.File.Delete(System.IO.Path.Combine(System.Environment.GetEnvironmentVariable("LOCALAPPDATA"),"CodexProxyTray","config.json"));}}'))
    & (Join-Path $env:SystemRoot 'Microsoft.NET\Framework64\v4.0.30319\csc.exe') /nologo /target:winexe ("/out:"+(Join-Path $fake 'CodexProxyTray.exe')) $code
    if($LASTEXITCODE -ne 0){throw 'Synthetic executable did not compile'}
    $receiptPath=Join-Path $fake 'package-receipt.json';$receipt=Get-Content -LiteralPath $receiptPath -Raw|ConvertFrom-Json;foreach($file in $receipt.files){$file.sha256=Hash (Join-Path $fake $file.name)};WriteJson $receiptPath $receipt
    $sums=@(Get-ChildItem -LiteralPath $fake|Where-Object Name -ne 'SHA256SUMS.txt'|ForEach-Object {"$(Hash $_.FullName)  $($_.Name)"});[IO.File]::WriteAllText((Join-Path $fake 'SHA256SUMS.txt'),(($sums -join "`n")+"`n"),[Text.UTF8Encoding]::new($false))
    $fakeZip=Join-Path $bad.directory 'fake.zip';Compress-Archive -LiteralPath @(Get-ChildItem -LiteralPath $fake|Select-Object -ExpandProperty FullName) -DestinationPath $fakeZip
    $job=Job $bad $fakeZip;$check=Installer $bad 'Prepare';Require ($check.WaitForExit(45000) -and $check.ExitCode -eq 0) 'synthetic_failure_package_validates_without_bypass'
    $installer=Installer $bad 'Apply';$bad.installer=$installer;WaitFor {Test-Path -LiteralPath (Join-Path $job 'handoff-ready.json')} 10 'Rollback installer handoff failed';Quit $bad
    Require ($installer.WaitForExit(60000) -and $installer.ExitCode -ne 0) 'unready_candidate_does_not_report_success'
    $result=Get-Content -LiteralPath (Join-Path $job 'result.json') -Raw|ConvertFrom-Json;Require ($result.phase -eq 'rolled_back') 'normal_failure_restores_previous_code'
    Require ((Hash $bad.exe) -eq (Hash (Join-Path $good.target 'CodexProxyTray.exe'))) 'rollback_restores_exact_original_binary'
    Require ((Hash (Join-Path $bad.data 'config.json')) -eq (Hash (Join-Path $job 'config.json'))) 'rollback_restores_missing_configuration_exactly'
    Copy-Item -LiteralPath (Join-Path $job 'result.json') -Destination (Join-Path $EvidenceDirectory 'rollback-result.json');StopCase $bad
    $traversal=Case 'traversal';$cases+=$traversal;$hostile=Join-Path $traversal.directory 'traversal.zip';Copy-Item -LiteralPath $zip -Destination $hostile;$archive=[IO.Compression.ZipFile]::Open($hostile,[IO.Compression.ZipArchiveMode]::Update);try{$entry=$archive.CreateEntry('../escape.txt');$writer=[IO.StreamWriter]::new($entry.Open());try{$writer.Write('must not escape')}finally{$writer.Dispose()}}finally{$archive.Dispose()};$job=Job $traversal $hostile;$check=Installer $traversal 'Prepare';Require ($check.WaitForExit(45000) -and $check.ExitCode -ne 0 -and -not (Test-Path -LiteralPath (Join-Path $traversal.directory 'app\escape.txt')) -and -not $traversal.parent.HasExited) 'zip_traversal_rejected_without_replacing_original';StopCase $traversal
    $report.status='passed'
} catch {$report.status='failed';$report['error']=$_.Exception.Message;$report['diagnostic_root']=$root;throw}
finally {
    foreach($case in $cases){try{StopCase $case}catch{$report.cleanup='retained: owned fixture still active'}}
    if($report.status -eq 'passed' -and $report.cleanup -ne 'retained: owned fixture still active'){
        $resolved=(Resolve-Path -LiteralPath $root).Path;$boundary=[IO.Path]::GetFullPath($env:RUNNER_TEMP);if(-not $resolved.StartsWith($boundary+'\',[StringComparison]::OrdinalIgnoreCase) -or ((Get-Item -LiteralPath $resolved).Attributes -band [IO.FileAttributes]::ReparsePoint)){throw 'Cleanup boundary mismatch'}
        Remove-Item -LiteralPath $resolved -Recurse -Force;$report.cleanup='passed'
    }elseif($report.cleanup -eq 'pending'){$report.cleanup='retained_for_diagnosis'}
    $report['finished_at']=[DateTime]::UtcNow.ToString('o');WriteJson (Join-Path $EvidenceDirectory 'updater-checks.json') $report
}
$report|ConvertTo-Json -Depth 6
