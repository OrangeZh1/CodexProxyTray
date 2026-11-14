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
$desktopShortcut=Join-Path ([Environment]::GetFolderPath('DesktopDirectory')) 'CodexProxyTray.lnk'
if(Test-Path -LiteralPath $EvidenceDirectory){throw 'Existing installer evidence is preserved.'}
[void][IO.Directory]::CreateDirectory($EvidenceDirectory)
$work=Join-Path $env:RUNNER_TEMP ('CodexProxyTray-install-qa-'+[Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($work)
$checks=[Collections.Generic.List[object]]::new()
$report=[ordered]@{schema=1;status='running';scope='Fresh GitHub-hosted runner only; per-user setup, registration and exact owned uninstall. No real profiles, services, UAC automation or global proxy changes.';checks=$checks;cleanup='pending';outcomes=[Collections.Generic.List[object]]::new();shortcuts=[Collections.Generic.List[object]]::new()}
$ownedData=$false;$ownedRoot=$false;$successfulInstall=$false
function Require([bool]$Condition,[string]$Name){if(-not $Condition){throw $Name};$checks.Add(@{name=$Name;status='passed'})}
function Hash([string]$Path){(Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()}
function WriteJson([string]$Path,$Value){[IO.File]::WriteAllText($Path,($Value|ConvertTo-Json -Depth 16),[Text.UTF8Encoding]::new($false))}
function RunInstaller([string]$File,[string[]]$Arguments=@('/S')){
    $info=[Diagnostics.ProcessStartInfo]::new($File);$info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.WindowStyle='Hidden';$info.WorkingDirectory=[IO.Path]::GetDirectoryName($File)
    $info.RedirectStandardOutput=$true;$info.RedirectStandardError=$true;$info.StandardOutputEncoding=[Text.Encoding]::Unicode
    if($Arguments.Count -gt 0 -and $Arguments[-1] -cmatch '^(?:/D=|_\?=)'){
        # NSIS consumes its final directory override as an unquoted command-line tail.
        $tail=$Arguments[-1];$prefix=@($Arguments|Select-Object -SkipLast 1)
        if($tail -match '[\r\n\x00"]' -or $tail.Length -le 3 -or @($prefix|Where-Object {$_ -cnotmatch '^(?:/S|/DESKTOP=[01])$'}).Count){throw 'invalid_controlled_nsis_directory_arguments'}
        $info.Arguments=$Arguments -join ' '
    }else{foreach($argument in $Arguments){$info.ArgumentList.Add($argument)}}
    $started=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds();$process=[Diagnostics.Process]::Start($info)
    $stdout=$process.StandardOutput.ReadToEndAsync();$stderr=$process.StandardError.ReadToEndAsync()
    try{
        if(-not $process.WaitForExit(120000)){throw 'installer_timeout_preserved'}
        CaptureOutcome $process.ExitCode $started
        # A resident child must not make diagnostic collection wait indefinitely.
        [void][Threading.Tasks.Task]::WaitAll(@($stdout,$stderr),2000)
        $diagnostic=(@($stdout,$stderr)|ForEach-Object {if($_.IsCompletedSuccessfully){$_.Result}else{'<diagnostic stream not completed>'}}) -join "`n"
        foreach($prefix in @($installRoot,$data,$work)){if($prefix){$diagnostic=$diagnostic.Replace($prefix,'<owned-path>')}};$diagnostic=[regex]::Replace($diagnostic,'S-\d-(?:\d+-)*\d+','<SID>')
        if($diagnostic.Length -gt 4096){$diagnostic=$diagnostic.Substring(0,4096)};$report.outcomes[-1]['bootstrap_output']=$diagnostic
        return $process.ExitCode
    }finally{$process.Dispose()}
}
function CaptureOutcome([int]$ExitCode,[long]$Started){
    $path=Join-Path $data 'installation-result-v1.json'
    if(Test-Path -LiteralPath $path){
        if((Get-Item -LiteralPath $path).Length -gt 16384){throw 'installer_result_too_large'}
        $value=Get-Content -LiteralPath $path -Raw|ConvertFrom-Json
        if($value.completed_at -lt $Started){$report.outcomes.Add(@{exit_code=$ExitCode;result_present=$false;stale_previous_result_preserved=$true});return}
        $text=[string]$value.error;foreach($prefix in @($installRoot,$data,$work)){if($prefix){$text=$text.Replace($prefix,'<owned-path>')}};$text=[regex]::Replace($text,'S-\d-(?:\d+-)*\d+','<SID>')
        $location=$null;if($value.error_location){$frames=@();foreach($frame in @($value.error_location.frames|Select-Object -First 6)){if($frame.function -is [string] -and $frame.function -cmatch '^(?:[A-Za-z][A-Za-z0-9_-]*|<ScriptBlock>)$' -and $frame.line -gt 0 -and $frame.line -lt 100000){$frames+=@{function=$frame.function;line=$frame.line}}};$location=@{line=$value.error_location.line;frames=$frames}}
        $report.outcomes.Add([ordered]@{action=$value.action;status=$value.status;phase=$value.phase;version=$value.version;exit_code=$ExitCode;error=$text;completed_at=$value.completed_at;error_location=$location})
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
function RegisteredUpgradeFixture([switch]$UnknownDesktopOff) {
    # v0.2.2 shipped as a portable ZIP. This is a synthetic registered layout
    # using its real, immutable code, not a claim that v0.2.2 had a Setup EXE.
    $oldVersion='0.2.2';$oldSource='568789e7e3e9d1096725e682e5da4f5a210b0530'
    $fixture=Join-Path $work ('registered-upgrade-'+[Guid]::NewGuid().ToString('N'));[void][IO.Directory]::CreateDirectory($fixture)
    $archive=Join-Path $fixture 'official-v0.2.2.zip'
    $uri='https://github.com/OrangeZh1/CodexProxyTray/releases/download/v0.2.2/CodexProxyTray-v0.2.2-windows-x64.zip'
    Invoke-WebRequest -Uri $uri -OutFile $archive -TimeoutSec 60
    Require ((Hash $archive) -ceq 'ce4cb875325eda9f0872ef304bd156c6baa99d0908b67021e79dc0952dc2ca2f') 'registered_upgrade_fixture_uses_pinned_real_old_code'
    $current=Join-Path $installRoot 'current';$old=Join-Path $fixture 'old-payload'
    [IO.Compression.ZipFile]::ExtractToDirectory($archive,$old)
    $oldReceipt=Get-Content -LiteralPath (Join-Path $old 'package-receipt.json') -Raw|ConvertFrom-Json
    Require ($oldReceipt.version -ceq $oldVersion -and $oldReceipt.source_commit -ceq $oldSource -and (Get-Item -LiteralPath (Join-Path $old 'CodexProxyTray.exe')).VersionInfo.ProductVersion -ceq $oldVersion) 'registered_upgrade_fixture_has_real_old_pe_version'
    $oldActive=Get-Content -LiteralPath (Join-Path $installRoot 'active-version.json') -Raw|ConvertFrom-Json
    $uninstallerHash=Hash (Join-Path $installRoot 'uninstall.exe')
    if($UnknownDesktopOff){AssertDesktop $true}
    $oldHelper=Get-Process -Id ([int]$oldActive.readiness.helper_pid) -ErrorAction Stop
    try {
        Require ($oldHelper.Path -ieq (Join-Path $current 'CodexProxyTray.exe')) 'registered_upgrade_fixture_quits_only_its_helper'
        $quit=RunApp $oldHelper.Path @('--quit');try{Require ($quit.WaitForExit(10000) -and $oldHelper.WaitForExit(20000)) 'registered_upgrade_fixture_graceful_quit'}finally{$quit.Dispose()}
    }finally{$oldHelper.Dispose()}
    # RUNNER_TEMP and LocalApplicationData can be on different volumes. Only
    # this synthetic fixture copies files; production switches remain atomic.
    $saved=Join-Path $fixture 'pre-fixture-current';[void][IO.Directory]::CreateDirectory($saved)
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
        if($UnknownDesktopOff){[IO.File]::WriteAllText($desktopShortcut,'Synthetic unknown desktop retained while turning preference off.');$unknownDesktopHash=Hash $desktopShortcut}
        $setupArguments=if($UnknownDesktopOff){@('/S','/DESKTOP=0')}else{@('/S')}
        Require ((RunInstaller $setup $setupArguments) -eq 0 -and $helper.WaitForExit(10000)) 'registered_layout_upgrade_uses_real_handoff_and_restart'
        if($UnknownDesktopOff){Require ((Hash $desktopShortcut) -ceq $unknownDesktopHash) 'upgrade_off_applies_new_choice_before_registration_and_retains_unknown_desktop';[IO.File]::Delete($desktopShortcut);AssertDesktop $false;Require ((RunInstaller $setup @('/S','/DESKTOP=1')) -eq 0) 'restore_owned_desktop_after_upgrade_off_fixture';AssertDesktop $true}
    }finally{$helper.Dispose()}
    $active=Get-Content -LiteralPath (Join-Path $installRoot 'active-version.json') -Raw|ConvertFrom-Json
    Require ($active.current.version -ceq $manifest.version -and $active.current.source_commit -ceq $env:GITHUB_SHA -and $active.previous.source_commit -ceq $oldSource -and (Get-Item -LiteralPath (Join-Path $installRoot 'previous\CodexProxyTray.exe')).VersionInfo.ProductVersion -ceq $oldVersion) 'registered_upgrade_commits_new_version_and_one_real_previous'
    Require ((Hash (Join-Path $installRoot 'uninstall.exe')) -ceq $uninstallerHash) 'registered_upgrade_preserves_existing_uninstaller_and_marker_contract'
    AssertRegistration
}
function RootSnapshot {
    $result=[ordered]@{};foreach($file in Get-ChildItem -LiteralPath (Join-Path $installRoot 'current') -File|Sort-Object Name){$result[$file.Name]=Hash $file.FullName};$result|ConvertTo-Json -Compress
}
function LegacySwitchingRecoveryFixture {
    Require (-not(Test-Path -LiteralPath $installRoot) -and -not(Test-Path -LiteralPath $reg) -and @(Get-Process -Name CodexProxyTray -ErrorAction SilentlyContinue).Count -eq 0) 'legacy024_recovery_starts_from_fresh_owned_code_root'
    $legacySetup=Join-Path $work 'official-v0.2.4-setup.exe';Invoke-WebRequest -Uri 'https://github.com/OrangeZh1/CodexProxyTray/releases/download/v0.2.4/CodexProxyTray-v0.2.4-windows-x64-setup.exe' -OutFile $legacySetup -TimeoutSec 60
    Require ((Hash $legacySetup) -ceq '3f64946dcf735d792ab462d1921e3cca0204a121c5ef9442269f7a2dc87f96d1' -and (Get-Item -LiteralPath $legacySetup).VersionInfo.ProductVersion -ceq '0.2.4') 'legacy024_recovery_uses_pinned_official_setup'
    Require ((RunInstaller $legacySetup @('/S','/DESKTOP=0')) -eq 0) 'legacy024_original_setup_installs_real_registered_payload'
    $current=Join-Path $installRoot 'current';$image=Join-Path $current 'CodexProxyTray.exe';$activePath=Join-Path $installRoot 'active-version.json';$journalPath=Join-Path $installRoot 'install-journal.json';$oldActive=Get-Content -LiteralPath $activePath -Raw|ConvertFrom-Json
    Require ($oldActive.current.version -ceq '0.2.4' -and $oldActive.current.source_commit -ceq '752f554d5e20741fb311b2042c51b04f50f2132d' -and (Hash $image) -ceq '3b9d91fc3b4b122fb8e7384b38105fb27dc3235b20e41efa24c4cf84c62494a2' -and $oldActive.readiness.main_rendered -and $oldActive.readiness.tray_rendered -and @($oldActive.current.files).Count -eq 10) 'legacy024_current_is_pinned_and_genuinely_healthy'
    foreach($file in @($oldActive.current.files)){Require ((Hash (Join-Path $current $file.name)) -ceq $file.sha256) 'legacy024_current_file_matches_original_active_receipt'}
    $configHashBefore=Hash (Join-Path $data 'config.json');$markerHashBefore=Hash (Join-Path $installRoot 'installation.json');$uninstallHashBefore=Hash (Join-Path $installRoot 'uninstall.exe');$activeHashBefore=Hash $activePath;$preJournalHash=Hash $journalPath;$before=RootSnapshot
    $raw=Join-Path $work 'official-v024-install-update-raw.ps1';Invoke-WebRequest -Uri 'https://raw.githubusercontent.com/OrangeZh1/CodexProxyTray/752f554d5e20741fb311b2042c51b04f50f2132d/src-tauri/src/install_update.ps1' -OutFile $raw -TimeoutSec 60
    Require ((Hash $raw) -ceq 'fef1557971c428390fa12b16aa313f2b98f09aff011f1d27183f5497f6f33d7e') 'legacy024_updater_is_pinned_original_source'
    $id=$PID.ToString()+'-'+[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds();$job=Join-Path (Join-Path $data 'updates') $id;[void][IO.Directory]::CreateDirectory($job);$candidate=Join-Path $installRoot ('.candidate-update-'+$id);$obsolete=Join-Path $installRoot ('.obsolete-update-'+$id)
    $archive=Join-Path $release ("CodexProxyTray-v$($manifest.version)-windows-x64.zip");$zipHash=Hash $archive;$sums=Get-Content -LiteralPath (Join-Path $release 'SHA256SUMS.txt');Require (@($sums|Where-Object {$_ -ceq ($zipHash+'  '+[IO.Path]::GetFileName($archive))}).Count -eq 1) 'legacy024_failure_fixture_uses_verified_new_release_package'
    [IO.File]::Copy($archive,(Join-Path $job 'package.zip'),$false);[IO.File]::Copy((Join-Path $release 'CodexProxyTray-update.json'),(Join-Path $job 'manifest.json'),$false);[IO.File]::WriteAllText((Join-Path $job 'installer.ps1'),[IO.File]::ReadAllText($raw,[Text.UTF8Encoding]::new($false,$true)),[Text.UTF8Encoding]::new($true))
    $parent=Get-Process -Id ([int]$oldActive.readiness.helper_pid) -ErrorAction Stop;$apply=$null
    function StartLegacyPhase([string]$Phase){$info=[Diagnostics.ProcessStartInfo]::new((Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'));$info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.WindowStyle='Hidden';$info.WorkingDirectory=$current;foreach($argument in @('-NoLogo','-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',(Join-Path $job 'installer.ps1'),'-Phase',$Phase,'-Job',$job)){$info.ArgumentList.Add($argument)};[Diagnostics.Process]::Start($info)}
    try{
        Require ($parent.Path -ieq $image) 'legacy024_failure_fixture_owns_exact_original_gui'
        WriteJson (Join-Path $job 'plan.json') @{schema=1;product='CodexProxyTray';version=$manifest.version;zip_sha256=$zipHash;manifest_sha256=(Hash (Join-Path $job 'manifest.json'));executable=$image;data_root=$data;parent_pid=$parent.Id;parent_created=$parent.StartTime.ToUniversalTime().ToFileTimeUtc().ToString()}
        $prepare=StartLegacyPhase 'Prepare';try{Require ($prepare.WaitForExit(90000) -and $prepare.ExitCode -eq 0) 'legacy024_original_prepare_builds_real_verified_candidate'}finally{$prepare.Dispose()}
        $apply=StartLegacyPhase 'Apply';$handoff=Join-Path $job 'handoff-ready.json';$deadline=[DateTime]::UtcNow.AddSeconds(20);while(-not(Test-Path -LiteralPath $handoff)){if($apply.HasExited -or [DateTime]::UtcNow -ge $deadline){throw 'legacy024_original_handoff_missing'};Start-Sleep -Milliseconds 100};$ready=Get-Content -LiteralPath $handoff -Raw|ConvertFrom-Json;Require ($ready.pid -eq $apply.Id -and $ready.state -ceq 'waiting-parent') 'legacy024_apply_reaches_real_waiting_parent_handoff'
        $quit=RunApp $image @('--quit');try{Require ($quit.WaitForExit(10000) -and $parent.WaitForExit(20000)) 'legacy024_original_gui_exits_normally_before_move'}finally{$quit.Dispose()}
        Require ($apply.WaitForExit(60000) -and $apply.ExitCode -ne 0) 'legacy024_updater_from_current_cwd_reproduces_failed_switch'
    }finally{if($apply){$apply.Dispose()};$parent.Dispose()}
    $journal=Get-Content -LiteralPath $journalPath -Raw|ConvertFrom-Json;$result=Get-Content -LiteralPath (Join-Path $job 'result.json') -Raw|ConvertFrom-Json;$proof=Get-Content -LiteralPath (Join-Path $job 'verified.json') -Raw|ConvertFrom-Json
    Require ($journal.state -ceq 'switching' -and $journal.recovery -ieq $job -and $journal.candidate -ieq $candidate -and $result.phase -ceq 'error' -and $result.version -ceq $manifest.version -and (RootSnapshot) -ceq $before -and (Hash $activePath) -ceq $activeHashBefore -and -not(Test-Path -LiteralPath (Join-Path $installRoot 'previous')) -and -not(Test-Path -LiteralPath $obsolete) -and @(Get-ChildItem -LiteralPath $candidate -Force).Count -eq 10) 'legacy024_failure_matches_untouched_current_slots_and_full_candidate'
    $switchHash=Hash $journalPath;$verifiedPath=Join-Path $job 'verified.json';$verifiedBytes=[IO.File]::ReadAllBytes($verifiedPath);$retirement=Join-Path $job 'candidate-retirement-v1.json';$evidence=Join-Path $job 'recovery-switching-journal.json'
    $unknown=Join-Path $candidate 'user-notes.txt';[IO.File]::WriteAllText($unknown,'owned synthetic candidate unknown file');try{Require ((RunInstaller $setup @('/S','/DESKTOP=0')) -ne 0 -and [IO.File]::ReadAllText($unknown) -ceq 'owned synthetic candidate unknown file' -and (RootSnapshot) -ceq $before -and (Hash $journalPath) -ceq $switchHash -and -not(Test-Path -LiteralPath $retirement)) 'legacy024_recovery_refuses_unknown_candidate_without_mutating_slots'}finally{[IO.File]::Delete($unknown)}
    try{$bad=[Text.Encoding]::UTF8.GetString($verifiedBytes)|ConvertFrom-Json;$bad.source_commit='0'*40;WriteJson $verifiedPath $bad;Require ((RunInstaller $setup @('/S','/DESKTOP=0')) -ne 0 -and (RootSnapshot) -ceq $before -and (Hash $journalPath) -ceq $switchHash -and -not(Test-Path -LiteralPath $retirement)) 'legacy024_recovery_refuses_tampered_verified_source'}finally{[IO.File]::WriteAllBytes($verifiedPath,$verifiedBytes)}
    $small=Join-Path $candidate 'QUICKSTART.txt';$smallBytes=[IO.File]::ReadAllBytes($small);try{[IO.File]::Delete($small);Require ((RunInstaller $setup @('/S','/DESKTOP=0')) -ne 0 -and (Hash $journalPath) -ceq $switchHash -and -not(Test-Path -LiteralPath $retirement)) 'legacy024_missing_candidate_file_without_retirement_is_refused'}finally{[IO.File]::WriteAllBytes($small,$smallBytes)}
    # Sharing permits validation reads but refuses deletion of the final allowlist
    # file. This interrupts real production cleanup after its durable declaration.
    $held=[IO.File]::Open((Join-Path $candidate 'SHA256SUMS.txt'),'Open','Read','Read');try{Require ((RunInstaller $setup @('/S','/DESKTOP=0')) -ne 0) 'legacy024_retirement_cleanup_interruption_is_reported'}finally{$held.Dispose()}
    $remaining=@(Get-ChildItem -LiteralPath $candidate -Force);Require ((Test-Path -LiteralPath $retirement) -and (Hash $evidence) -ceq $switchHash -and (Hash $journalPath) -ceq $switchHash -and $remaining.Count -gt 0 -and $remaining.Count -lt 10 -and (RootSnapshot) -ceq $before) 'legacy024_partial_cleanup_keeps_durable_retirement_and_original_switching_evidence'
    foreach($file in $remaining){$expected=@($proof.after|Where-Object name -CEQ $file.Name);Require ($expected.Count -eq 1 -and (Hash $file.FullName) -ceq $expected[0].sha256) 'legacy024_retired_subset_contains_only_exact_declared_files'}
    $retirementBytes=[IO.File]::ReadAllBytes($retirement);try{$bad=[Text.Encoding]::UTF8.GetString($retirementBytes)|ConvertFrom-Json;$bad.zip_sha256='0'*64;WriteJson $retirement $bad;Require ((RunInstaller $setup @('/S','/DESKTOP=0')) -ne 0 -and @(Get-ChildItem -LiteralPath $candidate -Force).Count -eq $remaining.Count -and (Hash $journalPath) -ceq $switchHash) 'legacy024_partial_cleanup_refuses_tampered_retirement'}finally{[IO.File]::WriteAllBytes($retirement,$retirementBytes)}
    Require ((RunInstaller $setup @('/S','/DESKTOP=0')) -eq 0) 'new_setup_resumes_legacy024_retirement_then_performs_normal_upgrade'
    $newActive=Get-Content -LiteralPath $activePath -Raw|ConvertFrom-Json;$retired=Get-Content -LiteralPath $retirement -Raw|ConvertFrom-Json
    Require ($newActive.current.version -ceq $manifest.version -and $newActive.current.source_commit -ceq $env:GITHUB_SHA -and $newActive.previous.source_commit -ceq $oldActive.current.source_commit -and (Get-Item -LiteralPath (Join-Path $installRoot 'previous\CodexProxyTray.exe')).VersionInfo.ProductVersion -ceq '0.2.4') 'legacy024_recovery_upgrade_commits_new_current_and_exact_one_old_previous'
    Require ($retired.schema -eq 1 -and $retired.product -ceq 'CodexProxyTray.CandidateRetirement' -and $retired.state -ceq 'candidate-retired-original-still-active' -and $retired.verified_role -ceq 'historical-only' -and $retired.journal_sha256 -ceq $switchHash -and $retired.active_sha256 -ceq $activeHashBefore -and $retired.pre_journal_sha256 -ceq $preJournalHash -and (Hash (Join-Path $job 'package.zip')) -ceq $zipHash -and (Hash $verifiedPath) -ceq $retired.verified_sha256 -and -not(Test-Path -LiteralPath $candidate) -and -not(Test-Path -LiteralPath $obsolete)) 'legacy024_failed_payload_and_historical_proofs_are_preserved_while_candidate_is_retired'
    Require ((Hash (Join-Path $data 'config.json')) -ceq $configHashBefore -and (Hash $logSentinel) -ceq $logHash -and (Hash $usageSentinel) -ceq $usageHash -and (Hash (Join-Path $installRoot 'installation.json')) -ceq $markerHashBefore -and (Hash (Join-Path $installRoot 'uninstall.exe')) -ceq $uninstallHashBefore) 'legacy024_recovery_preserves_configuration_logs_usage_and_original_installation_identity'
    $rootNames=@('current','previous','uninstall.exe','installation.json','active-version.json','install-journal.json','.current-update.lock');Require (@(Compare-Object ($rootNames|Sort-Object) (@(Get-ChildItem -LiteralPath $installRoot -Force|ForEach-Object Name)|Sort-Object)).Count -eq 0) 'legacy024_recovery_leaves_no_unknown_candidate_or_obsolete_code_slot'
    AssertRegistration;AssertDesktop $false;Require ((RunUninstaller) -eq 0 -and -not(Test-Path -LiteralPath $installRoot) -and -not(Test-Path -LiteralPath $reg)) 'legacy024_original_uninstaller_cleans_new_registered_code_after_recovery'
    Require ((Test-Path -LiteralPath $retirement) -and (Hash (Join-Path $job 'package.zip')) -ceq $zipHash -and (Hash $logSentinel) -ceq $logHash -and (Hash $usageSentinel) -ceq $usageHash) 'legacy024_uninstall_retains_failed_payload_recovery_evidence_and_user_data'
}
function ShortcutText([string]$Value){foreach($pair in @(@($work,'<fixture>'),@($local,'<LocalAppData>'),@($env:RUNNER_TEMP,'<RunnerTemp>'),@($env:USERPROFILE,'<UserProfile>'))){if($pair[0]){$Value=$Value.Replace($pair[0],$pair[1])}};$Value}
function ReadinessDiagnostics {
    $expected=Join-Path $installRoot 'current\CodexProxyTray.exe';$value=[ordered]@{source_root=(ShortcutText $installRoot);observed_at=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds();current_exe_exists=$false;processes=@();files=[ordered]@{}}
    if(-not $ownedData){$value.skipped='fixture_data_not_owned';return $value}
    $fixed=Join-Path $local 'Programs\CodexProxyTray';$rootPath=[IO.Path]::GetFullPath($installRoot)
    if($rootPath -ine $fixed -and -not $rootPath.StartsWith([IO.Path]::GetFullPath($work).TrimEnd('\')+'\',[StringComparison]::OrdinalIgnoreCase)){$value.skipped='fixture_code_root_outside_owned_paths';return $value}
    # Only the named synthetic fixture files below are read; never config or log/history bodies.
    $cursor=$data;while($cursor){if(Test-Path -LiteralPath $cursor){if((Get-Item -LiteralPath $cursor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint){$value.skipped='fixture_data_reparse_point';return $value}};$cursor=[IO.Path]::GetDirectoryName($cursor)}
    $value.current_exe_exists=Test-Path -LiteralPath $expected -PathType Leaf
    try{foreach($process in @(Get-CimInstance Win32_Process -Filter "Name='CodexProxyTray.exe'")){if($process.ExecutablePath -and [IO.Path]::GetFullPath($process.ExecutablePath) -ieq [IO.Path]::GetFullPath($expected)){$value.processes+=@{pid=$process.ProcessId;started_at=$process.CreationDate.ToUniversalTime().ToString('o');path=(ShortcutText $process.ExecutablePath)}}}}catch{$value.process_read_error=ShortcutText $_.Exception.Message}
    $specifications=[ordered]@{
        'ui-error.json'=@{limit=65536;fields=@('helper_pid','at','window','message')}
        'startup-error.txt'=@{limit=65536;fields=@()}
        'ui-ready-main.json'=@{limit=65536;fields=@('helper_pid','at','rendered','window')}
        'ui-ready-tray.json'=@{limit=65536;fields=@('helper_pid','at','rendered','window')}
        'status.json'=@{limit=131072;fields=@('helper_pid','updated_at','enabled','backend_ready','error')}
    }
    foreach($name in $specifications.Keys){
        $record=[ordered]@{exists=$false};$value.files[$name]=$record;$path=Join-Path $data $name
        try{
            if(-not(Test-Path -LiteralPath $path)){continue};$record.exists=$true;$item=Get-Item -LiteralPath $path -Force;$record.bytes=$item.Length;$limit=$specifications[$name].limit
            if($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -or $item.Length -gt $limit){$record.skipped='unsupported_type_or_size';continue}
            $stream=[IO.File]::OpenRead($path);$buffer=[byte[]]::new($limit+1);$length=0
            try{while($length -lt $buffer.Length){$read=$stream.Read($buffer,$length,$buffer.Length-$length);if($read -eq 0){break};$length+=$read}}finally{$stream.Dispose()}
            if($length -gt $limit){$record.skipped='grew_beyond_read_limit';continue};$text=[Text.UTF8Encoding]::new($false,$true).GetString($buffer,0,$length)
            if($name -ceq 'startup-error.txt'){$record.error=ShortcutText ($text.Substring(0,[Math]::Min(1000,$text.Length)));continue}
            $json=$text|ConvertFrom-Json;$record.selected=[ordered]@{}
            foreach($field in $specifications[$name].fields){$content=$json.$field;if($content -is [string]){$record.selected[$field]=ShortcutText ($content.Substring(0,[Math]::Min(1000,$content.Length)))}elseif($null -eq $content -or $content -is [bool] -or $content -is [int] -or $content -is [long] -or $content -is [double]){$record.selected[$field]=$content}}
        }catch{$record.read_error=$_.Exception.GetType().FullName;$record.read_hresult=('0x{0:X8}' -f ($_.Exception.HResult -band 0xffffffffL))}
    }
    $value
}
function ReadNativeShortcut([string]$Path){
    if(-not ('CodexProxyTray.InstallerShortcutNative' -as [type])){Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
using System.Text;
namespace CodexProxyTray {
 [ComImport,Guid("000214F9-0000-0000-C000-000000000046"),InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
 interface IShortcutReadW {
  [PreserveSig] int GetPath([Out,MarshalAs(UnmanagedType.LPWStr)] StringBuilder path,int characters,IntPtr findData,uint flags);
  [PreserveSig] int GetIDList(out IntPtr idList);
  [PreserveSig] int SetIDList(IntPtr idList);
  [PreserveSig] int GetDescription([Out,MarshalAs(UnmanagedType.LPWStr)] StringBuilder description,int characters);
  [PreserveSig] int SetDescription([MarshalAs(UnmanagedType.LPWStr)] string description);
  [PreserveSig] int GetWorkingDirectory([Out,MarshalAs(UnmanagedType.LPWStr)] StringBuilder directory,int characters);
  [PreserveSig] int SetWorkingDirectory([MarshalAs(UnmanagedType.LPWStr)] string directory);
  [PreserveSig] int GetArguments([Out,MarshalAs(UnmanagedType.LPWStr)] StringBuilder arguments,int characters);
 }
 public sealed class NativeShortcutValues {
  public int PathHResult,RawPathHResult,ArgumentsHResult,WorkingDirectoryHResult;
  public string Path,RawPath,Arguments,WorkingDirectory;
 }
 public sealed class InstallerMainWindow {public uint ProcessId;public bool Visible,Minimized;public int Width,Height;}
 public static class InstallerShortcutNative {
  delegate bool EnumWindowProc(IntPtr window,IntPtr parameter);
  [StructLayout(LayoutKind.Sequential)] struct Rect {public int left,top,right,bottom;}
  [DllImport("user32.dll",SetLastError=true)] static extern bool EnumWindows(EnumWindowProc callback,IntPtr parameter);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window,out uint pid);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr window,StringBuilder title,int maximum);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr window);
  [DllImport("user32.dll")] static extern bool IsIconic(IntPtr window);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr window,out Rect rect);
  public static InstallerMainWindow[] MainWindows(uint expectedPid){
   var matches=new System.Collections.Generic.List<InstallerMainWindow>();
   if(!EnumWindows((window,parameter)=>{uint pid;GetWindowThreadProcessId(window,out pid);if(pid!=expectedPid)return true;var title=new StringBuilder(256);GetWindowTextW(window,title,title.Capacity);if(!String.Equals(title.ToString(),"CodexProxyTray",StringComparison.Ordinal))return true;Rect rect;if(GetWindowRect(window,out rect))matches.Add(new InstallerMainWindow {ProcessId=pid,Visible=IsWindowVisible(window),Minimized=IsIconic(window),Width=rect.right-rect.left,Height=rect.bottom-rect.top});return true;},IntPtr.Zero))throw new System.ComponentModel.Win32Exception();return matches.ToArray();
  }
  public static NativeShortcutValues Read(string file){
   object root=null;
   try {
    root=Activator.CreateInstance(Type.GetTypeFromCLSID(new Guid("00021401-0000-0000-C000-000000000046"),true));
    // Only load the existing file read-only. Do not Resolve, Save or invoke it.
    ((IPersistFile)root).Load(file,0);
    var link=(IShortcutReadW)root;var path=new StringBuilder(32768);var raw=new StringBuilder(32768);var arguments=new StringBuilder(32768);var directory=new StringBuilder(32768);
    var result=new NativeShortcutValues();result.PathHResult=link.GetPath(path,path.Capacity,IntPtr.Zero,0);result.RawPathHResult=link.GetPath(raw,raw.Capacity,IntPtr.Zero,4);result.ArgumentsHResult=link.GetArguments(arguments,arguments.Capacity);result.WorkingDirectoryHResult=link.GetWorkingDirectory(directory,directory.Capacity);
    result.Path=path.ToString();result.RawPath=raw.ToString();result.Arguments=arguments.ToString();result.WorkingDirectory=directory.ToString();return result;
   } finally {if(root!=null&&Marshal.IsComObject(root))Marshal.FinalReleaseComObject(root);}
  }
 }
}
'@}
    [CodexProxyTray.InstallerShortcutNative]::Read($Path)
}
function RecordShortcut([string]$Kind,[string]$Path){
    $expected=Join-Path $installRoot 'current\CodexProxyTray.exe';$actual='';$arguments='';$targetHash=$null;$targetExists=$false;$wshError=$null
    try{$shell=New-Object -ComObject WScript.Shell;$link=$shell.CreateShortcut($Path);$actual=[string]$link.TargetPath;$arguments=[string]$link.Arguments;$targetExists=$actual -and (Test-Path -LiteralPath $actual -PathType Leaf);if($targetExists){$targetHash=Hash $actual}}catch{$wshError=ShortcutText $_.Exception.Message}
    $exists=Test-Path -LiteralPath $Path -PathType Leaf;$expectedHash=if(Test-Path -LiteralPath $expected -PathType Leaf){Hash $expected}else{$null}
    $value=[ordered]@{kind=$Kind;source_root=(ShortcutText $installRoot);link_exists=[bool]$exists;target_path=(ShortcutText $actual);arguments=(ShortcutText $arguments);expected_target=(ShortcutText $expected);target_exists=[bool]$targetExists;target_sha256=$targetHash;expected_sha256=$expectedHash;target_hash_matches=($null -ne $targetHash -and $targetHash -ceq $expectedHash);native_valid=$false}
    if($wshError){$value.wsh_read_error=$wshError}
    if($exists){try{
        $native=ReadNativeShortcut $Path
        $value.native_get_path_hresult=('0x{0:X8}' -f $native.PathHResult);$value.native_get_path=(ShortcutText $native.Path)
        $value.native_get_raw_path_hresult=('0x{0:X8}' -f $native.RawPathHResult);$value.native_get_raw_path=(ShortcutText $native.RawPath)
        $value.native_get_arguments_hresult=('0x{0:X8}' -f $native.ArgumentsHResult);$value.native_arguments=(ShortcutText $native.Arguments)
        $value.native_get_working_directory_hresult=('0x{0:X8}' -f $native.WorkingDirectoryHResult);$value.native_working_directory=(ShortcutText $native.WorkingDirectory)
        $value.native_path_matches=$native.Path -ieq $expected;$value.native_raw_path_matches=$native.RawPath -ieq $expected
        $value.native_target_exists=Test-Path -LiteralPath $native.Path -PathType Leaf;$nativeHash=if($value.native_target_exists){Hash $native.Path}else{$null};$value.native_target_sha256=$nativeHash
        $value.native_target_hash_matches=$null -ne $nativeHash -and $nativeHash -ceq $expectedHash
        $value.native_valid=$native.PathHResult -eq 0 -and $native.RawPathHResult -eq 0 -and $native.ArgumentsHResult -eq 0 -and $value.native_path_matches -and $value.native_raw_path_matches -and [string]::IsNullOrEmpty($native.Arguments) -and $value.native_target_exists -and $value.native_target_hash_matches
    }catch{$value.native_read_error=ShortcutText $_.Exception.Message}}
    $report.shortcuts.Add($value);$value
}
function AssertRegistration {
    $entry=Get-ItemProperty -LiteralPath $reg
    Require ($entry.DisplayName -ceq 'CodexProxyTray' -and $entry.DisplayVersion -ceq $manifest.version -and [IO.Path]::GetFullPath($entry.InstallLocation) -ieq $installRoot) 'per_user_installed_apps_registration'
    Require ($entry.UninstallString.Contains((Join-Path $installRoot 'uninstall.exe')) -and (Test-Path -LiteralPath (Join-Path $installRoot 'uninstall.exe'))) 'stable_uninstaller_outside_current'
    Require (-not (Test-Path -LiteralPath $machineReg)) 'no_machine_wide_registration'
    $diagnostic=RecordShortcut 'start_menu' $shortcut
    if($diagnostic.link_exists -and -not $diagnostic.native_valid){
        try{
            $fixedSource=Join-Path ([Environment]::GetFolderPath('Programs')) 'CodexProxyTray.lnk';$destination=Join-Path ([IO.Path]::GetFullPath($EvidenceDirectory)) 'custom-start-menu-link.lnk';$item=Get-Item -LiteralPath $shortcut -Force
            if([IO.Path]::GetFullPath($shortcut) -ine [IO.Path]::GetFullPath($fixedSource) -or $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -or $item.Length -gt 65536 -or (Test-Path -LiteralPath $destination)){throw 'shortcut_evidence_boundary_refused'}
            $sourceHash=Hash $shortcut;[IO.File]::Copy($shortcut,$destination,$false);if((Hash $destination) -cne $sourceHash){throw 'shortcut_evidence_copy_hash_mismatch'};$diagnostic.evidence_name='custom-start-menu-link.lnk';$diagnostic.evidence_sha256=$sourceHash
        }catch{$diagnostic.capture_error=ShortcutText $_.Exception.Message}
    }
    Require ($diagnostic.link_exists -and $diagnostic.native_valid) 'single_normal_start_menu_entry'
    Require (-not (Test-Path -LiteralPath $legacyShortcut)) 'no_second_notification_start_menu_entry'
    Require (-not (Test-Path -LiteralPath (Join-Path $startMenu 'codex-proxy-cli.lnk'))) 'private_cli_has_no_start_menu_entry'
}
function AssertDesktop([bool]$Wanted){
    $marker=Get-Content -LiteralPath (Join-Path $installRoot 'installation.json') -Raw|ConvertFrom-Json
    $preferencePath=Join-Path $data ('installation-preferences-v1\'+$marker.install_id+'.json')
    $preference=Get-Content -LiteralPath $preferencePath -Raw|ConvertFrom-Json
    Require (@($marker.PSObject.Properties.Name).Count -eq 6 -and @($preference.PSObject.Properties.Name).Count -eq 5 -and $preference.schema -eq 1 -and $preference.product -ceq 'CodexProxyTray.InstallationPreferences' -and $preference.install_id -ceq $marker.install_id -and $preference.root -ieq $installRoot -and $preference.desktop_shortcut -is [bool] -and $preference.desktop_shortcut -eq $Wanted) 'strict_data_sidecar_preserves_six_field_marker'
    $diagnostic=RecordShortcut 'desktop' $desktopShortcut
    if($Wanted){Require ($diagnostic.link_exists -and $diagnostic.native_valid) 'desktop_shortcut_is_normal_current_helper'}else{Require (-not(Test-Path -LiteralPath $desktopShortcut)) 'desktop_choice_off_removes_only_owned_entry'}
}
function OpenOwnedStartMenu {
    $diagnostic=RecordShortcut 'custom_start_menu_before_shell_open' $shortcut
    Require ($diagnostic.link_exists -and $diagnostic.native_valid) 'unicode_start_menu_validated_before_shell_execute'
    $linkHash=Hash $shortcut;$active=Get-Content -LiteralPath (Join-Path $installRoot 'active-version.json') -Raw|ConvertFrom-Json;$expected=Join-Path $installRoot 'current\CodexProxyTray.exe'
    Require ($active.schema -eq 1 -and $active.state -ceq 'active' -and $active.current.path -ieq (Join-Path $installRoot 'current') -and $active.current.version -ceq $manifest.version -and $active.readiness.native -ceq 'passed' -and $active.readiness.main_rendered -and $active.readiness.tray_rendered -and $active.readiness.helper_pid -gt 0) 'shortcut_shell_open_uses_current_committed_helper'
    $helper=Get-Process -Id ([int]$active.readiness.helper_pid) -ErrorAction Stop;$activation=$null
    try{
        [void]$helper.SafeHandle;$created=$helper.StartTime.ToUniversalTime().ToFileTimeUtc();$started=([DateTimeOffset]$helper.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds();$image=@($active.current.files|Where-Object name -CEQ 'CodexProxyTray.exe')
        Require (-not $helper.HasExited -and $helper.Path -ieq $expected -and $image.Count -eq 1 -and (Hash $expected) -ceq $image[0].sha256) 'shortcut_shell_open_binds_exact_live_image'
        $ready=Get-Content -LiteralPath (Join-Path $data 'ui-ready-main.json') -Raw|ConvertFrom-Json
        Require ($ready.window -ceq 'main' -and $ready.helper_pid -eq $helper.Id -and $ready.rendered -and $ready.at -ge $started) 'shortcut_shell_open_has_fresh_main_readiness'
        Require ((Hash $shortcut) -ceq $linkHash) 'shortcut_shell_open_link_unchanged_after_validation'
        $before=@([CodexProxyTray.InstallerShortcutNative]::MainWindows([uint32]$helper.Id));$info=[Diagnostics.ProcessStartInfo]::new($shortcut);$info.UseShellExecute=$true;$info.Verb='open';$info.WindowStyle='Normal';$activation=[Diagnostics.Process]::Start($info)
        $deadline=[DateTime]::UtcNow.AddSeconds(15);$visible=@()
        do{
            $helper.Refresh();if($helper.HasExited -or $helper.StartTime.ToUniversalTime().ToFileTimeUtc() -ne $created -or $helper.Path -ine $expected){throw 'shortcut_shell_open_helper_identity_changed'}
            $visible=@([CodexProxyTray.InstallerShortcutNative]::MainWindows([uint32]$helper.Id)|Where-Object {$_.Visible -and -not $_.Minimized -and $_.Width -gt 0 -and $_.Height -gt 0});if($visible.Count -eq 1){break};Start-Sleep -Milliseconds 100
        }while([DateTime]::UtcNow -lt $deadline)
        Require ($visible.Count -eq 1 -and $visible[0].ProcessId -eq $active.readiness.helper_pid) 'unicode_owned_start_menu_shell_execute_shows_same_main_helper'
        $report['custom_shortcut_shell_open']=[ordered]@{helper_pid=$helper.Id;created_filetime=[string]$created;image_sha256=$image[0].sha256;link_sha256=$linkHash;visible_before=@($before|Where-Object Visible).Count -gt 0;visible=$true;minimized=$false;width=$visible[0].Width;height=$visible[0].Height;activation='ShellExecute owned StartMenu LNK';foreground_not_required=$true}
    }finally{if($activation){$activation.Dispose()};$helper.Dispose()}
}

function DesktopChoices {
    AssertDesktop $true
    Require ((RunInstaller $setup @('/S','/DESKTOP=0')) -eq 0) 'same_root_desktop_choice_off_is_idempotent';AssertDesktop $false
    Require ((RunInstaller $setup @('/S','/DESKTOP=1')) -eq 0) 'same_root_desktop_choice_on_is_idempotent';AssertDesktop $true
    Require ((RunInstaller $setup @('/S','/DESKTOP=0')) -eq 0) 'owned_desktop_can_be_removed_before_unknown_fixture'
    $unknown='Synthetic unknown desktop shortcut bytes; must never be overwritten.';[IO.File]::WriteAllText($desktopShortcut,$unknown,[Text.UTF8Encoding]::new($false));$unknownHash=Hash $desktopShortcut
    try{
        $before=RootSnapshot;$regBefore=(Get-ItemProperty -LiteralPath $reg).CodexProxyTrayOwner
        Require ((RunInstaller $setup @('/S','/DESKTOP=0')) -eq 0 -and (Hash $desktopShortcut) -ceq $unknownHash) 'desktop_off_retains_unknown_entry'
        Require ((RunInstaller $setup @('/S','/DESKTOP=1')) -ne 0 -and (Hash $desktopShortcut) -ceq $unknownHash -and (RootSnapshot) -ceq $before -and (Get-ItemProperty -LiteralPath $reg).CodexProxyTrayOwner -ceq $regBefore) 'desktop_on_refuses_unknown_entry_without_replacing_code'
    }finally{if((Test-Path -LiteralPath $desktopShortcut) -and (Hash $desktopShortcut) -ceq $unknownHash){[IO.File]::Delete($desktopShortcut)}}
    Require ((RunInstaller $setup @('/S','/DESKTOP=1')) -eq 0) 'desktop_choice_recovers_after_owned_fixture_cleanup';AssertDesktop $true
    $marker=Get-Content -LiteralPath (Join-Path $installRoot 'installation.json') -Raw|ConvertFrom-Json;$path=Join-Path $data ('installation-preferences-v1\'+$marker.install_id+'.json');$original=[IO.File]::ReadAllBytes($path)
    try{$value=[Text.Encoding]::UTF8.GetString($original)|ConvertFrom-Json;$value|Add-Member -NotePropertyName unknown -NotePropertyValue $true;WriteJson $path $value;$unknownHash=Hash $path;$before=RootSnapshot;Require ((RunInstaller $setup @('/S','/DESKTOP=0')) -ne 0 -and (Hash $path) -ceq $unknownHash -and (RootSnapshot) -ceq $before) 'unknown_installation_preference_is_retained_and_refused'}finally{[IO.File]::WriteAllBytes($path,$original)}
}
function RefuseOtherRoot {
    $other=Join-Path $work ('forbidden-second-install-'+[Guid]::NewGuid().ToString('N'));$before=RootSnapshot;$entry=Get-ItemProperty -LiteralPath $reg
    $rejected=RunInstaller $setup @('/S',('/D='+$other));$outcome=$report.outcomes[-1]
    Require ($rejected -ne 0 -and $outcome.phase -ceq 'error' -and $outcome.error -match '原安装目录升级' -and -not(Test-Path -LiteralPath $other) -and (RootSnapshot) -ceq $before -and (Get-ItemProperty -LiteralPath $reg).CodexProxyTrayOwner -ceq $entry.CodexProxyTrayOwner -and (Get-ItemProperty -LiteralPath $reg).InstallLocation -ceq $entry.InstallLocation) 'registered_installation_cannot_move_or_create_second_root'
}
function RecoveryFileComparison {
    $scriptPath=Join-Path $work 'recovery-file-comparison.ps1';$nonce=[Guid]::NewGuid().ToString('N')
    $probeSource=@'
param([string]$Source,[string]$Nonce)
$ErrorActionPreference='Stop'
if($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted'){throw 'isolated_runner_required'}
$tokens=$null;$errors=$null;$ast=[Management.Automation.Language.Parser]::ParseFile($Source,[ref]$tokens,[ref]$errors);if($errors.Count){throw 'source_parse_failed'}
foreach($name in @('AppFail','RecoveryKeys','RecoveryFiles')){$definition=$ast.Find({param($node)$node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -ceq $name},$true);if(-not $definition){throw 'source_function_missing'};. ([scriptblock]::Create($definition.Extent.Text))}
$payload=@('CodexProxyTray.exe','codex-proxy-cli.exe','WebView2Loader.dll','LICENSE','THIRD_PARTY_NOTICES.md','flag-icons-license.txt','package-receipt.json','QUICKSTART.txt','fluent-icons-license.txt','SHA256SUMS.txt')
$hashRows=@($payload|ForEach-Object {@{name=$_;sha256=('a'*64)}});$jsonRows=ConvertFrom-Json -InputObject (ConvertTo-Json -InputObject $hashRows -Depth 4)
if($hashRows.Count -ne 10 -or $jsonRows.Count -ne 10){throw 'fixture_shape_invalid'}
$canonical=RecoveryFiles $jsonRows;if((RecoveryFiles $hashRows) -cne $canonical){throw 'dictionary_and_json_comparison_differs'}
[Array]::Reverse($hashRows);if((RecoveryFiles $hashRows) -cne $canonical){throw 'input_order_changes_comparison'}
$jsonRows[0].sha256='b'*64;if((RecoveryFiles $jsonRows) -ceq $canonical){throw 'changed_hash_not_rejected'}
@{schema=1;status='passed';nonce=$Nonce;version=$PSVersionTable.PSVersion.ToString()}|ConvertTo-Json -Compress
'@
    [IO.File]::WriteAllText($scriptPath,$probeSource,[Text.UTF8Encoding]::new($true));$info=[Diagnostics.ProcessStartInfo]::new((Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'));$info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.WindowStyle='Hidden';$info.RedirectStandardOutput=$true;$info.RedirectStandardError=$true
    foreach($argument in @('-NoLogo','-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',$scriptPath,'-Source',(Join-Path $PSScriptRoot '..\src-tauri\src\install_application.ps1'),'-Nonce',$nonce)){$info.ArgumentList.Add($argument)}
    $process=[Diagnostics.Process]::Start($info);$stdout=$process.StandardOutput.ReadToEndAsync();$stderr=$process.StandardError.ReadToEndAsync()
    try{Require ($process.WaitForExit(15000)) 'winps51_recovery_file_comparison_exits';Require ([Threading.Tasks.Task]::WaitAll(@($stdout,$stderr),2000)) 'winps51_recovery_file_comparison_streams_close';Require ($process.ExitCode -eq 0 -and $stdout.Result.Length -le 4096 -and -not $stderr.Result) 'winps51_recovery_file_comparison_succeeds';$result=$stdout.Result|ConvertFrom-Json;Require ($result.schema -eq 1 -and $result.status -ceq 'passed' -and $result.nonce -ceq $nonce -and $result.version -like '5.1.*') 'winps51_recovery_file_comparison_binds_source_and_nonce';$report.recovery_file_comparison=@{status='passed';version=$result.version;order_and_representation_independent=$true;changed_hash_rejected=$true}}
    finally{if(-not $process.HasExited){$process.Kill($false);[void]$process.WaitForExit(5000)};$process.Dispose()}
}
function PathBoundaries {
    RecoveryFileComparison
    # Load only the existing path guards, not either installation entry point.
    $repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'));$tokens=$null;$errors=$null
    foreach($source in @(@{file='install_update.ps1';names=@('Plain')},@{file='install_application.ps1';names=@('AppFail','Full','Within','PlainParent','InstallDirectory')})){
        $ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $repo ('src-tauri/src/'+$source.file)),[ref]$tokens,[ref]$errors);Require (-not $errors.Count) 'path_guard_source_parses'
        foreach($name in $source.names){$definition=$ast.Find({param($node)$node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -ceq $name},$true);if(-not $definition){throw 'missing_production_path_guard'};. ([scriptblock]::Create($definition.Extent.Text))}
    }
    $driveRoot=[IO.Path]::GetPathRoot($local)
    $invalid=@($driveRoot,($driveRoot.Substring(0,2)+'relative'),'\\server\share\app',('\\?\'+$driveRoot+'app'),('\\.\'+$driveRoot+'app'),(Join-Path $work 'app:stream'),(Join-Path $work 'trailing.'),(Join-Path $work 'NUL\app'),(Join-Path $work '..\escape'),(Join-Path $work ('bad'+[char]10+'path')),(Join-Path $work 'bad"path'),$data,(Join-Path $data 'code'),$local)
    foreach($path in $invalid){$rejected=$false;try{[void](InstallDirectory $path)}catch{$rejected=$true};Require $rejected 'unsafe_installation_path_rejected_before_mutation'}
    $target=Join-Path $work ('junction-target-'+[Guid]::NewGuid().ToString('N'));$junction=Join-Path $work ('junction-link-'+[Guid]::NewGuid().ToString('N'));[void][IO.Directory]::CreateDirectory($target)
    [void](New-Item -ItemType Junction -Path $junction -Target $target);$rejected=$false;try{[void](InstallDirectory (Join-Path $junction 'app'))}catch{$rejected=$true}
    Require ($rejected -and @(Get-ChildItem -LiteralPath $target -Force).Count -eq 0 -and ((Get-Item -LiteralPath $junction -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) 'reparse_installation_parent_refused_without_following_link'
    [IO.Directory]::Delete($junction);[IO.Directory]::Delete($target)
}
function RefuseAndRetain([string]$Name,[string]$Path,[string]$Value,[switch]$Uninstall){
    [IO.File]::WriteAllText($Path,$Value);$before=RootSnapshot
    try{$exitCode=if($Uninstall){RunUninstaller}else{RunInstaller $setup};Require ($exitCode -ne 0 -and (Test-Path -LiteralPath $Path) -and (RootSnapshot) -ceq $before) $Name}finally{if([IO.File]::ReadAllText($Path) -ceq $Value){[IO.File]::Delete($Path)}}
}
try {
    Require ((Test-Path -LiteralPath $setup -PathType Leaf) -and -not (Test-Path -LiteralPath $installRoot) -and -not (Test-Path -LiteralPath $data) -and -not (Test-Path -LiteralPath $reg) -and -not (Test-Path -LiteralPath $shortcut) -and -not (Test-Path -LiteralPath $legacyShortcut) -and -not (Test-Path -LiteralPath $desktopShortcut)) 'fresh_per_user_install_preflight'
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
    $rejected=RunInstaller $setup;$rejectedOutcome=$report.outcomes[-1]
    Require ($rejected -ne 0 -and $rejectedOutcome.status -ceq 'failed' -and $rejectedOutcome.phase -ceq 'error' -and $rejectedOutcome.error -match '目标目录非空且没有本产品归属记录' -and [IO.File]::ReadAllText($unknown) -ceq 'synthetic unknown root file' -and -not (Test-Path -LiteralPath $reg)) 'unknown_first_install_directory_refused_by_real_backend'
    [IO.File]::Delete($unknown);[IO.Directory]::Delete($installRoot)
    Require ((RunInstaller $setup) -eq 0) 'fresh_standard_install_succeeds'
    $successfulInstall=$true
    Require (@(Get-ChildItem -LiteralPath (Join-Path $installRoot 'current') -Force).Count -eq 10 -and -not (Test-Path -LiteralPath (Join-Path $installRoot 'current\uninstall.exe'))) 'current_has_exact_final_runtime_payload'
    $active=Get-Content -LiteralPath (Join-Path $installRoot 'active-version.json') -Raw|ConvertFrom-Json
    Require ($active.schema -eq 1 -and $active.current.version -ceq $manifest.version -and $active.current.path -ieq (Join-Path $installRoot 'current') -and $active.readiness.main_rendered -and $active.readiness.tray_rendered) 'initial_active_receipt_after_real_health'
    AssertRegistration
    DesktopChoices
    RefuseOtherRoot
    $before=RootSnapshot;$installId=(Get-Content -LiteralPath (Join-Path $installRoot 'installation.json') -Raw|ConvertFrom-Json).install_id
    Require ((RunInstaller $setup) -eq 0 -and (RootSnapshot) -ceq $before -and (Get-Content -LiteralPath (Join-Path $installRoot 'installation.json') -Raw|ConvertFrom-Json).install_id -ceq $installId) 'same_version_install_is_idempotent'
    Set-ItemProperty -LiteralPath $reg -Name DisplayVersion -Value '0.0.1'
    $sync=RunApp (Join-Path $installRoot 'current\CodexProxyTray.exe') @('--register-installation')
    try{Require ($sync.WaitForExit(10000) -and $sync.ExitCode -eq 0) 'explicit_owned_registration_sync'}finally{$sync.Dispose()}
    AssertRegistration
    RegisteredUpgradeFixture -UnknownDesktopOff
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
    $marker=Get-Content -LiteralPath (Join-Path $installRoot 'installation.json') -Raw|ConvertFrom-Json;$badPreferencePath=Join-Path $data ('installation-preferences-v1\'+$marker.install_id+'.json');$badPreference=Get-Content -LiteralPath $badPreferencePath -Raw|ConvertFrom-Json;$badPreference|Add-Member -NotePropertyName unknown -NotePropertyValue 'preserve during uninstall';WriteJson $badPreferencePath $badPreference;$badPreferenceHash=Hash $badPreferencePath
    Require ((RunUninstaller) -eq 0) 'owned_uninstall_succeeds'
    Require ((Hash $badPreferencePath) -ceq $badPreferenceHash) 'malformed_external_preferences_do_not_block_uninstall_or_get_deleted'
    Require (-not (Test-Path -LiteralPath $installRoot) -and -not (Test-Path -LiteralPath $reg) -and -not (Test-Path -LiteralPath $shortcut) -and -not (Test-Path -LiteralPath $desktopShortcut)) 'owned_code_and_registration_removed'
    Require ([IO.File]::ReadAllText($sentinel) -ceq 'synthetic preserve-user-data' -and (Hash (Join-Path $data 'config.json')) -ceq $configHash -and (Hash $logSentinel) -ceq $logHash -and (Hash $usageSentinel) -ceq $usageHash) 'uninstall_preserves_exact_configuration_log_and_usage_bytes'
    PathBoundaries
    $defaultRoot=$installRoot;$installRoot=Join-Path $work '自选安装 目录'
    $report.custom_install_cross_drive=[IO.Path]::GetPathRoot($installRoot) -ine [IO.Path]::GetPathRoot($defaultRoot)
    Require (-not(Test-Path -LiteralPath $installRoot) -and $installRoot.Contains('自选安装 目录')) 'fresh_custom_directory_is_chinese_space_path_on_local_drive'
    Require ((RunInstaller $setup @('/S','/DESKTOP=1',('/D='+$installRoot))) -eq 0) 'custom_directory_real_install_succeeds'
    AssertRegistration;AssertDesktop $true;OpenOwnedStartMenu;RefuseOtherRoot
    RegisteredUpgradeFixture
    AssertDesktop $true
    Require ((RunUninstaller) -eq 0 -and -not(Test-Path -LiteralPath $installRoot) -and -not(Test-Path -LiteralPath $reg) -and -not(Test-Path -LiteralPath $shortcut) -and -not(Test-Path -LiteralPath $desktopShortcut)) 'custom_directory_upgrade_and_owned_desktop_uninstall_succeed'
    Require ((Hash (Join-Path $data 'config.json')) -ceq $configHash -and (Hash $logSentinel) -ceq $logHash -and (Hash $usageSentinel) -ceq $usageHash) 'custom_directory_uninstall_preserves_exact_user_data'
    # Exercise the actual published installer and its own generated uninstaller,
    # rather than substituting a legacy payload into a new installer layout.
    $installRoot=$defaultRoot
    Require (-not(Test-Path -LiteralPath $installRoot) -and -not(Test-Path -LiteralPath $reg) -and -not(Test-Path -LiteralPath $shortcut) -and -not(Test-Path -LiteralPath $desktopShortcut) -and @(Get-Process -Name CodexProxyTray -ErrorAction SilentlyContinue).Count -eq 0) 'genuine_v023_installation_starts_from_fresh_default_code_root'
    $oldSetup=Join-Path $work 'official-v0.2.3-setup.exe'
    Invoke-WebRequest -Uri 'https://github.com/OrangeZh1/CodexProxyTray/releases/download/v0.2.3/CodexProxyTray-v0.2.3-windows-x64-setup.exe' -OutFile $oldSetup -TimeoutSec 60
    Require ((Hash $oldSetup) -ceq '4f30ab45c5ab2e7b94e6660fff889ccf1d1b858612387bb5e5a363627cd845f2' -and (Get-Item -LiteralPath $oldSetup).VersionInfo.ProductVersion -ceq '0.2.3') 'genuine_v023_setup_is_pinned_official_release'
    Require ((RunInstaller $oldSetup) -eq 0) 'genuine_v023_setup_installs_its_real_uninstaller'
    $oldActive=Get-Content -LiteralPath (Join-Path $installRoot 'active-version.json') -Raw|ConvertFrom-Json
    Require ($oldActive.current.version -ceq '0.2.3' -and $oldActive.current.source_commit -ceq '5eca4970518f0b8801fa959fc1fb4955f74e6c23' -and (Get-Item -LiteralPath (Join-Path $installRoot 'current\CodexProxyTray.exe')).VersionInfo.ProductVersion -ceq '0.2.3' -and -not(Test-Path -LiteralPath $desktopShortcut)) 'genuine_v023_code_is_healthy_and_has_no_desktop_shortcut'
    $legacyUninstallerHash=Hash (Join-Path $installRoot 'uninstall.exe');$legacyMarkerHash=Hash (Join-Path $installRoot 'installation.json')
    Require ((RunInstaller $setup @('/S','/DESKTOP=1')) -eq 0) 'genuine_v023_installation_upgrades_with_desktop_choice'
    $upgraded=Get-Content -LiteralPath (Join-Path $installRoot 'active-version.json') -Raw|ConvertFrom-Json
    Require ($upgraded.current.version -ceq $manifest.version -and $upgraded.current.source_commit -ceq $env:GITHUB_SHA -and $upgraded.previous.source_commit -ceq $oldActive.current.source_commit -and (Hash (Join-Path $installRoot 'uninstall.exe')) -ceq $legacyUninstallerHash -and (Hash (Join-Path $installRoot 'installation.json')) -ceq $legacyMarkerHash) 'genuine_v023_uninstaller_and_exact_marker_survive_new_version_upgrade'
    AssertRegistration;AssertDesktop $true
    Require ((RunUninstaller) -eq 0) 'genuine_v023_uninstaller_removes_upgraded_installation'
    Require (-not(Test-Path -LiteralPath $installRoot) -and -not(Test-Path -LiteralPath $reg) -and -not(Test-Path -LiteralPath $shortcut) -and -not(Test-Path -LiteralPath $desktopShortcut)) 'genuine_v023_uninstaller_removes_new_desktop_menu_code_and_registration'
    Require ((Hash (Join-Path $data 'config.json')) -ceq $configHash -and (Hash $logSentinel) -ceq $logHash -and (Hash $usageSentinel) -ceq $usageHash) 'genuine_v023_uninstall_preserves_exact_synthetic_user_data'
    LegacySwitchingRecoveryFixture
    $report.status='passed'
}catch{$report.status='failed';$report.error=$_.Exception.Message;$report.diagnostic_root=$work;throw}
finally {
    if($report.status -eq 'failed'){try{$report.readiness_diagnostics=ReadinessDiagnostics}catch{$report.readiness_diagnostics=@{capture_error=(ShortcutText $_.Exception.Message)}}}
    # Do not try a reverse operation after an uncertain or rejected installation.
    # Failed code/data/registration are retained as recovery evidence on this disposable runner.
    $report.cleanup=if($report.status -eq 'passed'){'product_uninstalled_user_data_retained_as_verified'}else{'retained_for_diagnostics'}
    $report.finished_at=[DateTime]::UtcNow.ToString('o');WriteJson (Join-Path $EvidenceDirectory 'installer-checks.json') $report
}
