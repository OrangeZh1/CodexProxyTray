param([Parameter(Mandatory=$true)][string]$ReleaseDirectory,[Parameter(Mandatory=$true)][string]$EvidenceDirectory)
$ErrorActionPreference='Stop'
if(-not $IsWindows -or $env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted'){throw 'Run standard installer acceptance only on a fresh GitHub-hosted Windows runner.'}
$release=(Resolve-Path -LiteralPath $ReleaseDirectory).Path
$manifest=Get-Content -LiteralPath (Join-Path $release 'CodexProxyTray-update.json') -Raw|ConvertFrom-Json
$setup=Join-Path $release ("CodexProxyTray-v$($manifest.version)-windows-x64-setup.exe")
$local=[Environment]::GetFolderPath('LocalApplicationData')
$installRoot=Join-Path $local 'Programs\CodexProxyTray'
$data=Join-Path $local 'CodexProxyTray'
$reg='HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexProxyTray'
$machineReg='HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexProxyTray'
$startMenu=[Environment]::GetFolderPath('Programs')
$shortcut=Join-Path $startMenu 'CodexProxyTray.lnk'
$legacyShortcut=Join-Path $startMenu 'CodexProxyTray 更新通知.lnk'
if(Test-Path -LiteralPath $EvidenceDirectory){throw 'Existing installer evidence is preserved.'}
[void][IO.Directory]::CreateDirectory($EvidenceDirectory)
$work=Join-Path $env:RUNNER_TEMP ('CodexProxyTray-install-qa-'+[Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($work)
$checks=[Collections.Generic.List[object]]::new()
$report=[ordered]@{schema=1;status='running';scope='Fresh GitHub-hosted runner only; per-user setup, registration and exact owned uninstall. No real profiles, services, UAC automation or global proxy changes.';checks=$checks;cleanup='pending';outcomes=[Collections.Generic.List[object]]::new()}
$ownedData=$false;$ownedRoot=$false;$successfulInstall=$false
function Require([bool]$Condition,[string]$Name){if(-not $Condition){throw $Name};$checks.Add(@{name=$Name;status='passed'})}
function Hash([string]$Path){(Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()}
function WriteJson([string]$Path,$Value){[IO.File]::WriteAllText($Path,($Value|ConvertTo-Json -Depth 16),[Text.UTF8Encoding]::new($false))}
function RunInstaller([string]$File,[string[]]$Arguments=@('/S')){
    $info=[Diagnostics.ProcessStartInfo]::new($File);$info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.WindowStyle='Hidden';$info.WorkingDirectory=[IO.Path]::GetDirectoryName($File)
    foreach($argument in $Arguments){$info.ArgumentList.Add($argument)}
    $process=[Diagnostics.Process]::Start($info)
    try{if(-not $process.WaitForExit(120000)){throw 'installer_timeout_preserved'};CaptureOutcome $process.ExitCode;return $process.ExitCode}finally{$process.Dispose()}
}
function CaptureOutcome([int]$ExitCode){
    $path=Join-Path $data 'installation-result-v1.json'
    if(Test-Path -LiteralPath $path){
        if((Get-Item -LiteralPath $path).Length -gt 16384){throw 'installer_result_too_large'}
        $value=Get-Content -LiteralPath $path -Raw|ConvertFrom-Json
        $text=[string]$value.error;foreach($prefix in @($installRoot,$data,$work)){if($prefix){$text=$text.Replace($prefix,'<owned-path>')}};$text=[regex]::Replace($text,'S-\d-(?:\d+-)*\d+','<SID>')
        $report.outcomes.Add([ordered]@{action=$value.action;status=$value.status;phase=$value.phase;version=$value.version;exit_code=$ExitCode;error=$text;completed_at=$value.completed_at})
    }else{$report.outcomes.Add(@{exit_code=$ExitCode;result_present=$false})}
}
function RunApp([string]$File,[string[]]$Arguments){
    $info=[Diagnostics.ProcessStartInfo]::new($File);$info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.WindowStyle='Hidden';$info.WorkingDirectory=[IO.Path]::GetDirectoryName($File)
    foreach($argument in $Arguments){$info.ArgumentList.Add($argument)}
    [Diagnostics.Process]::Start($info)
}
function RunUninstaller {
    $source=Join-Path $installRoot 'uninstall.exe'
    $copy=Join-Path $work ('uninstall-'+[Guid]::NewGuid().ToString('N')+'.exe')
    Copy-Item -LiteralPath $source -Destination $copy
    Require ((Hash $copy) -ceq (Hash $source)) 'owned_uninstaller_copy_verified'
    RunInstaller $copy @('/S',('_?='+$installRoot))
}
function RegisteredUpgradeFixture {
    # v0.2.2 shipped as a portable ZIP. This is a synthetic registered layout
    # using its real, immutable code, not a claim that v0.2.2 had a Setup EXE.
    $oldVersion='0.2.2';$oldSource='568789e7e3e9d1096725e682e5da4f5a210b0530'
    $archive=Join-Path $work 'official-v0.2.2.zip'
    $uri='https://github.com/OrangeZh1/CodexProxyTray/releases/download/v0.2.2/CodexProxyTray-v0.2.2-windows-x64.zip'
    Invoke-WebRequest -Uri $uri -OutFile $archive -TimeoutSec 60
    Require ((Hash $archive) -ceq 'ce4cb875325eda9f0872ef304bd156c6baa99d0908b67021e79dc0952dc2ca2f') 'registered_upgrade_fixture_uses_pinned_real_old_code'
    $current=Join-Path $installRoot 'current';$old=Join-Path $work 'old-payload'
    [IO.Compression.ZipFile]::ExtractToDirectory($archive,$old)
    $oldReceipt=Get-Content -LiteralPath (Join-Path $old 'package-receipt.json') -Raw|ConvertFrom-Json
    Require ($oldReceipt.version -ceq $oldVersion -and $oldReceipt.source_commit -ceq $oldSource -and (Get-Item -LiteralPath (Join-Path $old 'CodexProxyTray.exe')).VersionInfo.ProductVersion -ceq $oldVersion) 'registered_upgrade_fixture_has_real_old_pe_version'
    $oldActive=Get-Content -LiteralPath (Join-Path $installRoot 'active-version.json') -Raw|ConvertFrom-Json
    $oldHelper=Get-Process -Id ([int]$oldActive.readiness.helper_pid) -ErrorAction Stop
    try {
        Require ($oldHelper.Path -ieq (Join-Path $current 'CodexProxyTray.exe')) 'registered_upgrade_fixture_quits_only_its_helper'
        $quit=RunApp $oldHelper.Path @('--quit');try{Require ($quit.WaitForExit(10000) -and $oldHelper.WaitForExit(20000)) 'registered_upgrade_fixture_graceful_quit'}finally{$quit.Dispose()}
    }finally{$oldHelper.Dispose()}
    # RUNNER_TEMP and LocalApplicationData can be on different volumes. Only
    # this synthetic fixture copies files; production switches remain atomic.
    $saved=Join-Path $work 'pre-fixture-current';[void][IO.Directory]::CreateDirectory($saved)
    $files=@(Get-ChildItem -LiteralPath $current -Force)
    Require ($files.Count -eq 10 -and $oldActive.current.files.Count -eq 10 -and @($files|Where-Object {$_.PSIsContainer -or $_.Attributes -band [IO.FileAttributes]::ReparsePoint}).Count -eq 0) 'registered_fixture_current_exact_file_boundary'
    foreach($file in $files){$expected=@($oldActive.current.files|Where-Object name -CEQ $file.Name);Require ($expected.Count -eq 1 -and (Hash $file.FullName) -ceq $expected[0].sha256) 'registered_fixture_current_owned_hash';[IO.File]::Copy($file.FullName,(Join-Path $saved $file.Name),$false)}
    foreach($file in $files){[IO.File]::Delete($file.FullName)}
    foreach($file in Get-ChildItem -LiteralPath $old -File){[IO.File]::Copy($file.FullName,(Join-Path $current $file.Name),$false)}
    $oldActive.current.version=$oldVersion;$oldActive.current.source_commit=$oldSource
    $oldActive.current.files=@(Get-ChildItem -LiteralPath $current -File|ForEach-Object {@{name=$_.Name;sha256=Hash $_.FullName}})
    $helper=RunApp (Join-Path $current 'CodexProxyTray.exe') @('--background')
    try {
        $deadline=[DateTime]::UtcNow.AddSeconds(45)
        do {
            $ready=$true;foreach($window in @('main','tray')){try{$value=Get-Content -LiteralPath (Join-Path $data ('ui-ready-'+$window+'.json')) -Raw|ConvertFrom-Json;if($value.helper_pid -ne $helper.Id -or -not $value.rendered -or $value.at -lt ([DateTimeOffset]$helper.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()){$ready=$false}}catch{$ready=$false}}
            if($ready){break};if($helper.HasExited){throw 'old_fixture_helper_exited'};Start-Sleep -Milliseconds 200
        }while([DateTime]::UtcNow -lt $deadline)
        Require $ready 'registered_upgrade_fixture_old_code_really_renders'
        $oldActive.readiness.helper_pid=$helper.Id;$oldActive.committed_at=[DateTime]::UtcNow.ToString('o')
        WriteJson (Join-Path $installRoot 'active-version.json') $oldActive
        Set-ItemProperty -LiteralPath $reg -Name DisplayVersion -Value $oldVersion
        Require ((RunInstaller $setup) -eq 0 -and $helper.WaitForExit(10000)) 'registered_layout_upgrade_uses_real_handoff_and_restart'
    }finally{$helper.Dispose()}
    $active=Get-Content -LiteralPath (Join-Path $installRoot 'active-version.json') -Raw|ConvertFrom-Json
    Require ($active.current.version -ceq $manifest.version -and $active.current.source_commit -ceq $env:GITHUB_SHA -and $active.previous.source_commit -ceq $oldSource -and (Get-Item -LiteralPath (Join-Path $installRoot 'previous\CodexProxyTray.exe')).VersionInfo.ProductVersion -ceq $oldVersion) 'registered_upgrade_commits_new_version_and_one_real_previous'
    AssertRegistration
}
function RootSnapshot {
    $result=[ordered]@{};foreach($file in Get-ChildItem -LiteralPath (Join-Path $installRoot 'current') -File|Sort-Object Name){$result[$file.Name]=Hash $file.FullName};$result|ConvertTo-Json -Compress
}
function AssertRegistration {
    $entry=Get-ItemProperty -LiteralPath $reg
    Require ($entry.DisplayName -ceq 'CodexProxyTray' -and $entry.DisplayVersion -ceq $manifest.version -and [IO.Path]::GetFullPath($entry.InstallLocation) -ieq $installRoot) 'per_user_installed_apps_registration'
    Require ($entry.UninstallString.Contains((Join-Path $installRoot 'uninstall.exe')) -and (Test-Path -LiteralPath (Join-Path $installRoot 'uninstall.exe'))) 'stable_uninstaller_outside_current'
    Require (-not (Test-Path -LiteralPath $machineReg)) 'no_machine_wide_registration'
    $shell=New-Object -ComObject WScript.Shell;$link=$shell.CreateShortcut($shortcut)
    Require ($link.TargetPath -ieq (Join-Path $installRoot 'current\CodexProxyTray.exe') -and [string]::IsNullOrEmpty($link.Arguments)) 'single_normal_start_menu_entry'
    Require (-not (Test-Path -LiteralPath $legacyShortcut)) 'no_second_notification_start_menu_entry'
    Require (-not (Test-Path -LiteralPath (Join-Path $startMenu 'codex-proxy-cli.lnk'))) 'private_cli_has_no_start_menu_entry'
}
function RefuseAndRetain([string]$Name,[string]$Path,[string]$Value,[switch]$Uninstall){
    [IO.File]::WriteAllText($Path,$Value);$before=RootSnapshot
    try{$exitCode=if($Uninstall){RunUninstaller}else{RunInstaller $setup};Require ($exitCode -ne 0 -and (Test-Path -LiteralPath $Path) -and (RootSnapshot) -ceq $before) $Name}finally{if([IO.File]::ReadAllText($Path) -ceq $Value){[IO.File]::Delete($Path)}}
}
try {
    Require ((Test-Path -LiteralPath $setup -PathType Leaf) -and -not (Test-Path -LiteralPath $installRoot) -and -not (Test-Path -LiteralPath $data) -and -not (Test-Path -LiteralPath $reg) -and -not (Test-Path -LiteralPath $shortcut) -and -not (Test-Path -LiteralPath $legacyShortcut)) 'fresh_per_user_install_preflight'
    Require (@(Get-Process -Name CodexProxyTray -ErrorAction SilentlyContinue).Count -eq 0) 'no_preexisting_gui_touched'
    [void][IO.Directory]::CreateDirectory($data);$ownedData=$true
    WriteJson (Join-Path $data 'config.json') @{launch_target='none';network_enabled=$false;auto_check_updates=$false;connectivity_enabled=$false;traffic_enabled=$false}
    $sentinel=Join-Path $data 'installer-user-data-sentinel.txt';[IO.File]::WriteAllText($sentinel,'synthetic preserve-user-data')
    [void][IO.Directory]::CreateDirectory((Join-Path $data 'logs'))
    $logSentinel=Join-Path $data 'logs\installer-preserved.log';[IO.File]::WriteAllText($logSentinel,'synthetic existing log bytes')
    $usageSentinel=Join-Path $data 'installer-usage-sentinel.txt';[IO.File]::WriteAllText($usageSentinel,'synthetic existing usage bytes')
    $logHash=Hash $logSentinel;$usageHash=Hash $usageSentinel
    [void][IO.Directory]::CreateDirectory($installRoot);$ownedRoot=$true
    $unknown=Join-Path $installRoot 'user-notes.txt';[IO.File]::WriteAllText($unknown,'synthetic unknown root file')
    Require ((RunInstaller $setup) -ne 0 -and [IO.File]::ReadAllText($unknown) -ceq 'synthetic unknown root file' -and -not (Test-Path -LiteralPath $reg)) 'unknown_first_install_directory_refused'
    [IO.File]::Delete($unknown);[IO.Directory]::Delete($installRoot)
    Require ((RunInstaller $setup) -eq 0) 'fresh_standard_install_succeeds'
    $successfulInstall=$true
    Require (@(Get-ChildItem -LiteralPath (Join-Path $installRoot 'current') -Force).Count -eq 10 -and -not (Test-Path -LiteralPath (Join-Path $installRoot 'current\uninstall.exe'))) 'current_has_exact_final_runtime_payload'
    $active=Get-Content -LiteralPath (Join-Path $installRoot 'active-version.json') -Raw|ConvertFrom-Json
    Require ($active.schema -eq 1 -and $active.current.version -ceq $manifest.version -and $active.current.path -ieq (Join-Path $installRoot 'current') -and $active.readiness.main_rendered -and $active.readiness.tray_rendered) 'initial_active_receipt_after_real_health'
    AssertRegistration
    $before=RootSnapshot;$installId=(Get-Content -LiteralPath (Join-Path $installRoot 'installation.json') -Raw|ConvertFrom-Json).install_id
    Require ((RunInstaller $setup) -eq 0 -and (RootSnapshot) -ceq $before -and (Get-Content -LiteralPath (Join-Path $installRoot 'installation.json') -Raw|ConvertFrom-Json).install_id -ceq $installId) 'same_version_install_is_idempotent'
    Set-ItemProperty -LiteralPath $reg -Name DisplayVersion -Value '0.0.1'
    $sync=RunApp (Join-Path $installRoot 'current\CodexProxyTray.exe') @('--register-installation')
    try{Require ($sync.WaitForExit(10000) -and $sync.ExitCode -eq 0) 'explicit_owned_registration_sync'}finally{$sync.Dispose()}
    AssertRegistration
    RegisteredUpgradeFixture
    RefuseAndRetain 'unknown_current_file_refused' (Join-Path $installRoot 'current\user-notes.txt') 'synthetic unknown current file'
    RefuseAndRetain 'unknown_root_file_refused' (Join-Path $installRoot 'user-notes.txt') 'synthetic unknown root file'
    foreach($file in @('installation.json','install-journal.json')){
        $path=Join-Path $installRoot $file;$bytes=[IO.File]::ReadAllBytes($path)
        try{$value=[Text.Encoding]::UTF8.GetString($bytes)|ConvertFrom-Json;$value.schema=999;WriteJson $path $value;$before=RootSnapshot;Require ((RunInstaller $setup) -ne 0 -and (RootSnapshot) -ceq $before) ('unknown_'+$file+'_refused')}finally{[IO.File]::WriteAllBytes($path,$bytes)}
    }
    $activePath=Join-Path $installRoot 'active-version.json';$activeBytes=[IO.File]::ReadAllBytes($activePath)
    try {
        $value=[Text.Encoding]::UTF8.GetString($activeBytes)|ConvertFrom-Json
        $value.recovery_references+=@((Join-Path $installRoot 'current\CodexProxyTray.exe'));WriteJson $activePath $value
        $before=RootSnapshot
        Require ((RunInstaller $setup) -ne 0 -and (RunUninstaller) -ne 0 -and (RootSnapshot) -ceq $before -and (Test-Path -LiteralPath $reg)) 'protected_code_recovery_reference_blocks_upgrade_and_uninstall'
    }finally{[IO.File]::WriteAllBytes($activePath,$activeBytes)}
    $configPath=Join-Path $data 'config.json';$originalConfig=[IO.File]::ReadAllBytes($configPath);$shim=$null;$ping=$null;$ownsPing=$false
    try {
        $config=[Text.Encoding]::UTF8.GetString($originalConfig)|ConvertFrom-Json
        $pingPath=Join-Path $env:SystemRoot 'System32\PING.EXE'
        $config|Add-Member -NotePropertyName cli_command -NotePropertyValue @($pingPath,'-t','127.0.0.1') -Force;WriteJson $configPath $config
        $info=[Diagnostics.ProcessStartInfo]::new((Join-Path $installRoot 'current\codex-proxy-cli.exe'));$info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.WindowStyle='Hidden';$info.RedirectStandardOutput=$true;$info.RedirectStandardError=$true
        $info.Environment['PATH']=Join-Path $env:SystemRoot 'System32';$info.Environment['APPDATA']=Join-Path $work 'empty-roaming'
        $shim=[Diagnostics.Process]::Start($info);$stdout=$shim.StandardOutput.ReadToEndAsync();$stderr=$shim.StandardError.ReadToEndAsync();$deadline=[DateTime]::UtcNow.AddSeconds(10)
        do {$child=Get-CimInstance Win32_Process -Filter "Name='PING.EXE' AND ParentProcessId=$($shim.Id)"|Where-Object {$_.ExecutablePath -ieq $pingPath}|Select-Object -First 1;if($child){$ping=Get-Process -Id $child.ProcessId;[void]$ping.SafeHandle;break};if($shim.HasExited){throw 'synthetic_cli_hold_exited'};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $deadline)
        Require ($null -ne $ping -and $ping.Path -ieq $pingPath -and $ping.StartTime.ToUniversalTime() -ge $shim.StartTime.ToUniversalTime() -and $ping.StartTime.ToUniversalTime().ToString('yyyyMMddHHmmss.ffffff') -ceq $child.CreationDate.ToUniversalTime().ToString('yyyyMMddHHmmss.ffffff')) 'only_owned_loopback_cli_fixture'
        $ownsPing=$true
        $before=RootSnapshot;Require ((RunInstaller $setup) -ne 0 -and -not $shim.HasExited -and -not $ping.HasExited -and (RootSnapshot) -ceq $before) 'active_cli_is_not_stopped_or_overwritten'
    } finally {
        if($ping){if($ownsPing -and -not $ping.HasExited){$ping.Kill();[void]$ping.WaitForExit(5000)};$ping.Dispose()}
        if($shim){if(-not $shim.WaitForExit(10000)){throw 'owned_cli_fixture_cleanup_blocked'};$shim.Dispose()}
        [IO.File]::WriteAllBytes($configPath,$originalConfig)
    }
    $journalPath=Join-Path $installRoot 'install-journal.json';$journalBytes=[IO.File]::ReadAllBytes($journalPath)
    try {
        $marker=Get-Content -LiteralPath (Join-Path $installRoot 'installation.json') -Raw|ConvertFrom-Json
        WriteJson $journalPath @{schema=1;product='CodexProxyTray.Installation';state='unregistered';install_id=$marker.install_id;current=(Join-Path $installRoot 'current');previous=(Join-Path $installRoot 'previous');marker_sha256=(Hash (Join-Path $installRoot 'installation.json'));active_sha256=('0'*64)}
        $before=RootSnapshot
        Require ((RunUninstaller) -ne 0 -and (RootSnapshot) -ceq $before -and (Test-Path -LiteralPath $reg)) 'uninstall_resume_rejects_mismatched_receipt_before_deleting_code'
    }finally{[IO.File]::WriteAllBytes($journalPath,$journalBytes)}
    RefuseAndRetain 'uninstall_unknown_code_retained' (Join-Path $installRoot 'current\user-notes.txt') 'synthetic unknown current file' -Uninstall
    $configHash=Hash (Join-Path $data 'config.json')
    Require ((RunUninstaller) -eq 0) 'owned_uninstall_succeeds'
    Require (-not (Test-Path -LiteralPath $installRoot) -and -not (Test-Path -LiteralPath $reg) -and -not (Test-Path -LiteralPath $shortcut)) 'owned_code_and_registration_removed'
    Require ([IO.File]::ReadAllText($sentinel) -ceq 'synthetic preserve-user-data' -and (Hash (Join-Path $data 'config.json')) -ceq $configHash -and (Hash $logSentinel) -ceq $logHash -and (Hash $usageSentinel) -ceq $usageHash) 'uninstall_preserves_exact_configuration_log_and_usage_bytes'
    $report.status='passed'
}catch{$report.status='failed';$report.error=$_.Exception.Message;$report.diagnostic_root=$work;throw}
finally {
    # Do not try a reverse operation after an uncertain or rejected installation.
    # Failed code/data/registration are retained as recovery evidence on this disposable runner.
    $report.cleanup=if($report.status -eq 'passed'){'product_uninstalled_user_data_retained_as_verified'}else{'retained_for_diagnostics'}
    $report.finished_at=[DateTime]::UtcNow.ToString('o');WriteJson (Join-Path $EvidenceDirectory 'installer-checks.json') $report
}
