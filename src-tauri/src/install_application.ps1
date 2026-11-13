param(
    [ValidateSet('Install','Uninstall')][string]$Action='Install',
    [string]$InstallRoot,
    [string]$PayloadZip,
    [string]$ZipSha256,
    [string]$Version,
    [string]$SourceCommit,
    [string]$UninstallerPath,
    [string]$UpdateEnginePath,
    [string]$ResultPath,
    [string]$PlanPath,
    [switch]$Silent
)
$ErrorActionPreference='Stop'
$exitCode=2;$outerLock=$null;$codeLock=$null;$applicationJob=$null;$applicationResult=$null;$healthy=$false;$removalStarted=$false;$bootstrapPhase='load_engine'
function AppFail([string]$Text){throw $Text}
function Full([string]$Path){if(-not $Path -or $Path.Contains([char]0) -or $Path.Contains('"')){AppFail '安装路径无效。'};[IO.Path]::GetFullPath($Path).TrimEnd('\')}
function Within([string]$Path,[string]$Directory){$Path.StartsWith($Directory.TrimEnd('\')+'\',[StringComparison]::OrdinalIgnoreCase)}
function PlainParent([string]$Path){$cursor=Full $Path;while(-not(Test-Path -LiteralPath $cursor)){$parent=[IO.Path]::GetDirectoryName($cursor);if(-not $parent -or $parent -eq $cursor){AppFail '安装目录的上级路径无效。'};$cursor=$parent};Plain $cursor}
function Event([string]$State){if(-not $applicationJob){return};$record=@{at=[DateTime]::UtcNow.ToString('o');action=$Action;phase=$State};[IO.File]::AppendAllText((Join-Path $applicationJob 'events.jsonl'),(($record|ConvertTo-Json -Compress)+"`r`n"),[Text.UTF8Encoding]::new($false));Durable (Join-Path $applicationJob 'phase.json') $record}
function SaveOutcome([string]$State,[string]$Message){
    $status=if($State -in @('installed','idempotent','uninstalled')){'passed'}elseif($State -like '*cleanup_pending'){'partial'}else{'failed'}
    $value=@{schema=1;product='CodexProxyTray.Installation';action=$Action;status=$status;phase=$State;version=$Version;root=$installRoot;job=$applicationJob;error=$Message;completed_at=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds();optional_service='unchanged'}
    if($applicationJob){Event $State;Durable (Join-Path $applicationJob 'result.json') $value}
    $latest=Join-Path $data 'installation-result-v1.json';if(Test-Path -LiteralPath $latest){$old=Json $latest;if($old.schema -ne 1 -or $old.product -cne 'CodexProxyTray.Installation'){AppFail '最近安装结果的归属不明，原文件已保留。'}};Durable $latest $value
    if($ResultPath){$destination=Full $ResultPath;PlainParent $destination;if(Test-Path -LiteralPath $destination){AppFail '调用方结果文件已存在，未覆盖。'};Durable $destination $value};$script:applicationResult=$value
}
function Arguments([string[]]$Values){($Values|ForEach-Object {if($_.Contains('"') -or $_.Contains([char]0) -or $_.EndsWith('\')){AppFail '安装命令参数无效。'};'"'+$_+'"'}) -join ' '}
function ChildEnvironment($Info,[switch]$UpdateScript){
    if($UpdateScript){$Info.EnvironmentVariables['CODEX_PROXY_UPDATER_MODULES_PRESENT']=if($originalModulesPresent){'1'}else{'0'};$Info.EnvironmentVariables['CODEX_PROXY_UPDATER_MODULES']=[string]$originalModulePath;$Info.EnvironmentVariables['CODEX_PROXY_UPDATER_POLICY_PRESENT']=if($originalPolicyPresent){'1'}else{'0'};$Info.EnvironmentVariables['CODEX_PROXY_UPDATER_POLICY']=[string]$originalPolicy}
    else{if($originalModulesPresent){$Info.EnvironmentVariables['PSModulePath']=[string]$originalModulePath}else{$Info.EnvironmentVariables.Remove('PSModulePath')};if($originalPolicyPresent){$Info.EnvironmentVariables['PSExecutionPolicyPreference']=[string]$originalPolicy}else{$Info.EnvironmentVariables.Remove('PSExecutionPolicyPreference')};foreach($name in @('CODEX_PROXY_UPDATER_MODULES_PRESENT','CODEX_PROXY_UPDATER_MODULES','CODEX_PROXY_UPDATER_POLICY_PRESENT','CODEX_PROXY_UPDATER_POLICY')){$Info.EnvironmentVariables.Remove($name)}}
}
function StartOwned([string]$File,[string[]]$Values,[switch]$UpdateScript){Plain $File;$info=[Diagnostics.ProcessStartInfo]::new($File);$info.Arguments=Arguments $Values;$info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.WindowStyle='Hidden';$info.WorkingDirectory=[IO.Path]::GetDirectoryName($File);ChildEnvironment $info -UpdateScript:$UpdateScript;[Diagnostics.Process]::Start($info)}
function RunOwned([string]$File,[string[]]$Values,[int]$Seconds=30){$process=StartOwned $File $Values;try{if(-not $process.WaitForExit($Seconds*1000)){AppFail '本次安装子进程仍在执行，已保留现场，请勿重复启动安装。'};if($process.ExitCode -ne 0){AppFail ('安装子进程未完成（退出码 '+$process.ExitCode+'）。')}}finally{$process.Dispose()}}
function SystemPowerShell {Join-Path ([Environment]::GetFolderPath('Windows')) 'System32\WindowsPowerShell\v1.0\powershell.exe'}
function StartUpdate([string]$PhaseValue,[string]$JobPath){StartOwned (SystemPowerShell) @('-NoLogo','-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',(Join-Path $JobPath 'installer.ps1'),'-Phase',$PhaseValue,'-Job',$JobPath) -UpdateScript}
function InstallPlan([string]$Path){
    Plain $Path;if((Get-Item -LiteralPath $Path).Length -gt 16384){AppFail '安装调用计划超过读取上限。'};$values=@{};$section=$false
    foreach($line in ([IO.File]::ReadAllText($Path) -split '\r?\n')){if(-not $line.Trim()){continue};if($line -ceq '[Application]'){if($section){AppFail '安装调用计划含重复分节。'};$section=$true;continue};if(-not $section -or $line -cnotmatch '^([A-Za-z][A-Za-z0-9]*)=(.*)$' -or $values.ContainsKey($Matches[1])){AppFail '安装调用计划格式无效。'};$values[$Matches[1]]=$Matches[2]}
    $keys=@('Action','InstallRoot','PayloadZip','ZipSha256','Version','SourceCommit','UninstallerPath','UpdateEnginePath','ResultPath','Silent');if($values.Count -ne $keys.Count -or @($values.Keys|Where-Object {$_ -cnotin $keys}).Count -or $values.Action -cnotin @('Install','Uninstall') -or $values.Silent -cnotin @('0','1') -or (Full $values.UpdateEnginePath) -ine $UpdateEnginePath){AppFail '安装调用计划与当前处理程序不匹配。'};$values
}
function ExactGone([int]$ProcessId,[long]$Created,[int]$Seconds){$end=[DateTime]::UtcNow.AddSeconds($Seconds);do{$process=Get-Process -Id $ProcessId -ErrorAction SilentlyContinue;try{if(-not $process -or $process.StartTime.ToUniversalTime().ToFileTimeUtc() -ne $Created){return $true}}finally{if($process){$process.Dispose()}};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $end);return $false}
function Helpers([switch]$RejectForeign){
    $matches=@();foreach($process in @(Get-CimInstance Win32_Process -Filter "name='CodexProxyTray.exe'")){if(-not $process.ExecutablePath){AppFail '某个助手进程的路径无法核验，未停止或替换任何程序。'};if($process.ExecutablePath -ieq $exe){$matches+=,$process}elseif($RejectForeign){AppFail '另一目录的助手仍在运行，请先从该助手正常退出；原便携目录不会自动移动或删除。'}};if($matches.Count -gt 1){AppFail '当前安装目录存在多个助手进程，请先正常退出。'};@($matches)
}
function NoCliUse {foreach($process in @(Get-CimInstance Win32_Process -Filter "name='codex-proxy-cli.exe'")){if(-not $process.ExecutablePath){AppFail '无法核验某个 CLI 入口的路径，安装操作已停止。'};if((Within $process.ExecutablePath $target) -or (Within $process.ExecutablePath $previous)){AppFail '当前安装目录的 Codex CLI 仍在使用，请先结束对应 CLI；其他 ChatGPT 和 CLI 进程不会被停止。'}}}
function StopRootHelper {
    $owned=@(Helpers);if(-not $owned.Count){return};$process=Get-Process -Id $owned[0].ProcessId -ErrorAction Stop
    try{if($process.Path -ine $exe){AppFail '助手进程身份已变化，未发送退出请求。'};$created=$process.StartTime.ToUniversalTime().ToFileTimeUtc();RunOwned $exe @('--quit') 15;if(-not(ExactGone $process.Id $created 25)){AppFail '当前助手未正常退出，代码和资料均已保留。'}}finally{$process.Dispose()}
}
function TakeCodeLock {$script:codeLock=[IO.File]::Open((Join-Path $installRoot '.current-update.lock'),'OpenOrCreate','ReadWrite','None')}
function ReleaseCodeLock {if($codeLock){$codeLock.Dispose();$script:codeLock=$null}}
function Marker {
    $value=Json (Join-Path $installRoot 'installation.json');$keys=@('schema','product','install_id','owner_sid','root','uninstaller_sha256');$names=@($value.PSObject.Properties.Name)
    if($names.Count -ne $keys.Count -or @($names|Where-Object {$_ -cnotin $keys}).Count -or $value.schema -ne 1 -or $value.product -cne 'CodexProxyTray.Installation' -or $value.install_id -cnotmatch '^[a-f0-9]{32}$' -or $value.owner_sid -cne $ownerSid -or (Full $value.root) -ine $installRoot -or $value.uninstaller_sha256 -cnotmatch '^[a-f0-9]{64}$' -or (Hash $uninstaller) -cne $value.uninstaller_sha256){AppFail '安装归属记录不匹配，原目录已保留。'};$value
}
function Slot([string]$Path,$Files,[switch]$Partial){
    if(@($Files).Count -ne $payload.Count -or @(Compare-Object ($payload|Sort-Object) (@($Files.name)|Sort-Object)).Count){AppFail '代码版本的精确文件清单无效。'}
    if($Partial){if(-not(Test-Path -LiteralPath $Path)){return};Plain $Path;if(-not @(Get-ChildItem -LiteralPath $Path -Force).Count){return};$actual=@(Inventory $Path);foreach($file in $actual){$expected=@($Files|Where-Object name -CEQ $file.name);if($expected.Count -ne 1 -or $expected[0].sha256 -cne $file.sha256){AppFail '卸载中的代码文件已变化，现场已保留。'}}}else{Verify $Path $Files}
}
function References($Active){
    foreach($reference in @($Active.recovery_references)+@($Active.historical_receipts)){
        if(-not $reference){continue};$path=Full ([string]$reference)
        if((Within $path $installRoot) -or (-not(Within $path (Join-Path $data 'updates')) -and -not(Within $path (Join-Path $data 'installations')))){AppFail '恢复引用尚未解析或仍保护安装代码，安装收尾被阻挡。'}
        if(-not(Test-Path -LiteralPath $path -PathType Leaf)){AppFail '恢复引用的材料缺失，原记录已保留。'};Plain $path
        if([IO.Path]::GetFileName($path) -cnotin @('config.json','events-v1.json','pre-active-version.json','pre-install-journal.json','receipt-lifecycle.json','result.json')){AppFail '恢复引用包含未知材料，未执行清理。'}
    }
}
function RootLayout($Journal,[string]$InitialCandidate=''){
    $allowed=@('current','previous','uninstall.exe','installation.json','active-version.json','install-journal.json','.current-update.lock');if($InitialCandidate){$allowed+=,[IO.Path]::GetFileName($InitialCandidate)}
    foreach($item in @(Get-ChildItem -LiteralPath $installRoot -Force)){Plain $item.FullName;if($item.Name -cnotin $allowed){AppFail '安装根目录存在未知文件或未结束的暂存版本，已保留并停止。'};if($item.PSIsContainer -ne (($item.Name -cin @('current','previous')) -or ($InitialCandidate -and $item.FullName -ieq $InitialCandidate))){AppFail '安装目录项类型不匹配。'}}
    $lockPath=Join-Path $installRoot '.current-update.lock';if((Test-Path -LiteralPath $lockPath) -and (Get-Item -LiteralPath $lockPath).Length -ne 0){AppFail '安装锁文件内容异常，已保留。'}
}
function Owned([switch]$Removing){
    $marker=Marker;$active=Json $activePath;$journal=Json $journalPath;RootLayout $journal
    if($journal.schema -ne 1){AppFail '安装事务记录格式版本无效，已保留。'}
    if('product' -cin @($journal.PSObject.Properties.Name)){
        if($journal.product -cne 'CodexProxyTray.Installation' -or $journal.install_id -cne $marker.install_id -or ($journal.root -and (Full $journal.root) -ine $installRoot) -or ($journal.owner_sid -and $journal.owner_sid -cne $ownerSid)){AppFail '安装事务的产品或用户归属不匹配。'}
    }elseif((Full $journal.current) -ine $target -or (Full $journal.previous) -ine $previous){AppFail '更新事务记录的代码路径不匹配。'}
    if($active.schema -ne 1 -or $active.state -cne 'active' -or (Full $active.current.path) -ine $target -or $active.current.version -cnotmatch '^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$' -or $active.readiness.native -cne 'passed' -or $active.readiness.main_rendered -ne $true -or $active.readiness.tray_rendered -ne $true){AppFail '活动版本或健康回执不完整，已保留。'}
    $resumeRemoval=($Removing -and $journal.state -ceq 'unregistered' -and $journal.product -ceq 'CodexProxyTray.Installation' -and $journal.install_id -ceq $marker.install_id)
    if($journal.state -cnotin @('installed','installed-partial','active','complete','registration-pending','cleanup-blocked','unregister-pending','unregistered')){AppFail '存在未完成安装事务，请先处理恢复记录。'}
    if($journal.state -ceq 'cleanup-blocked' -and ($journal.candidate -or $journal.obsolete)){AppFail '已有安装收尾仍引用暂存版本，未重新部署。'}
    if(-not $Removing -and $journal.state -cin @('unregister-pending','unregistered')){AppFail '已有卸载尚未收尾，未开始新的安装。'}
    if($journal.state -cin @('unregister-pending','unregistered')){if($journal.marker_sha256 -cnotmatch '^[a-f0-9]{64}$' -or $journal.active_sha256 -cnotmatch '^[a-f0-9]{64}$' -or (Hash (Join-Path $installRoot 'installation.json')) -cne $journal.marker_sha256 -or (Hash $activePath) -cne $journal.active_sha256){AppFail '卸载恢复摘要与当前归属回执不一致，未删除代码。'}}
    Slot $target $active.current.files -Partial:$resumeRemoval
    if(Test-Path -LiteralPath $previous){if((Full $active.previous.path) -ine $previous){AppFail '上一版本的目录归属不明。'};Slot $previous $active.previous.files -Partial:$resumeRemoval}
    if(-not $resumeRemoval){$receipt=Json (Join-Path $target 'package-receipt.json');if($receipt.schema -ne 1 -or $receipt.product -cne 'CodexProxyTray' -or $receipt.version -cne $active.current.version -or $receipt.source_commit -cne $active.current.source_commit -or (Get-Item -LiteralPath $exe).VersionInfo.ProductVersion -cne $active.current.version){AppFail '当前程序来源或版本与回执不一致。'}}
    References $active;[pscustomobject]@{marker=$marker;active=$active;journal=$journal;removing=[bool]$resumeRemoval}
}
function Register {Event 'registering';RunOwned $exe @('--register-installation') 20}
function InitialRecord {
    $journal=Json $journalPath
    if($journal.schema -ne 1 -or $journal.product -cne 'CodexProxyTray.Installation' -or $journal.operation -cne 'first-install' -or $journal.owner_sid -cne $ownerSid -or (Full $journal.root) -ine $installRoot -or $journal.install_id -cnotmatch '^[a-f0-9]{32}$' -or $journal.version -cne $Version -or $journal.source_commit -cne $SourceCommit -or $journal.zip_sha256 -cne $ZipSha256 -or $journal.state -cnotin @('preparing','prepared','awaiting-readiness','registration-pending')){AppFail '首次安装恢复记录无法确认，已保留。'}
    $preparedCandidate=Join-Path $installRoot ('.candidate-install-'+$journal.install_id);if((Full $journal.candidate) -ine $preparedCandidate){AppFail '首次安装暂存路径不匹配。'};RootLayout $journal $preparedCandidate
    [pscustomobject]@{journal=$journal;candidate=$preparedCandidate}
}
function Initialise {
    $existingJournal=if(Test-Path -LiteralPath $journalPath){Json $journalPath}else{$null}
    if($existingJournal){
        $prepared=InitialRecord;$journal=$prepared.journal;if('files' -cnotin @($journal.PSObject.Properties.Name)){$journal|Add-Member -NotePropertyName files -NotePropertyValue @()};$script:candidate=$prepared.candidate
    }else{
        if(-not $rootInitiallyEmpty -or @(Helpers).Count){AppFail '目标目录非空、正在使用或没有本产品归属记录，未覆盖。'}
        $script:candidate=Join-Path $installRoot ('.candidate-install-'+[Guid]::NewGuid().ToString('N'));$id=[IO.Path]::GetFileName($candidate).Substring(19)
        $journal=@{schema=1;product='CodexProxyTray.Installation';operation='first-install';install_id=$id;owner_sid=$ownerSid;root=$installRoot;version=$Version;source_commit=$SourceCommit;zip_sha256=$ZipSha256;uninstaller_sha256=(Hash $UninstallerPath);state='preparing';candidate=$candidate;files=@();recovery=$applicationJob;at=[DateTime]::UtcNow.ToString('o')};Durable $journalPath $journal
    }
    if(Test-Path -LiteralPath $uninstaller){if((Hash $uninstaller) -cne $journal.uninstaller_sha256){AppFail '卸载器与首次安装记录不一致。'}}else{if((Hash $UninstallerPath) -cne $journal.uninstaller_sha256){AppFail '恢复安装需要原安装器的卸载组件。'};CopyDurable $UninstallerPath $uninstaller}
    if(Test-Path -LiteralPath $target){if($journal.state -cnotin @('awaiting-readiness','registration-pending') -or -not $journal.files){AppFail '当前目录缺少已验证的首次安装状态。'};Slot $target $journal.files}
    else{
        if(Test-Path -LiteralPath $candidate){if(-not $journal.files){AppFail '首次安装暂存尚未验证，现场已保留，请先处理恢复记录。'};Slot $candidate $journal.files}
        else{[void][IO.Directory]::CreateDirectory($candidate);Event 'verifying-payload';$package=ExpandPayload $PayloadZip $candidate $Version $SourceCommit;$journal.files=@($package.files);$journal.state='prepared';Durable $journalPath $journal}
        SafeMove $candidate $target;$journal.state='awaiting-readiness';Durable $journalPath $journal
    }
    if(Test-Path -LiteralPath $activePath){$active=Json $activePath;if($active.schema -ne 1 -or $active.state -cne 'active' -or $active.current.version -cne $Version -or (Full $active.current.path) -ine $target -or $active.readiness.native -cne 'passed' -or $active.readiness.main_rendered -ne $true -or $active.readiness.tray_rendered -ne $true){AppFail '已有活动回执无法确认，未覆盖。'};Slot $target $active.current.files}
    else{
        Event 'awaiting-readiness';$running=@(Helpers -RejectForeign);$new=if($running.Count){Get-Process -Id $running[0].ProcessId -ErrorAction Stop}else{StartHelper '--background --after-update'}
        try{if(-not(Ready $new)){AppFail '新安装的助手未通过真实界面就绪检查，文件和恢复资料已保留。'}
            $active=@{schema=1;state='active';current=@{path=$target;version=$Version;source_commit=$SourceCommit;files=@($journal.files)};previous=@{path=$previous;files=@()};readiness=@{native='passed';main_rendered=$true;tray_rendered=$true;helper_pid=$new.Id};recovery_references=@();historical_receipts=@();committed_at=[DateTime]::UtcNow.ToString('o');optional_service='unchanged'};Durable $activePath $active
        }finally{$new.Dispose()}
    }
    $script:healthy=$true
    $marker=@{schema=1;product='CodexProxyTray.Installation';install_id=$journal.install_id;owner_sid=$ownerSid;root=$installRoot;uninstaller_sha256=$journal.uninstaller_sha256};Durable (Join-Path $installRoot 'installation.json') $marker
    try{Register;$journal.state='installed';Durable $journalPath $journal;SaveOutcome 'installed' $null;$script:exitCode=0}catch{$journal.state='registration-pending';Durable $journalPath $journal;throw}
}
function Upgrade($OwnedState){
    NoCliUse
    if([version]$Version -lt [version]$OwnedState.active.current.version){AppFail '目标版本低于当前健康版本，未执行降级。'}
    if($Version -ceq $OwnedState.active.current.version){
        if($SourceCommit -cne $OwnedState.active.current.source_commit){AppFail '同版本的编译来源不同，未覆盖已安装程序。'};$script:healthy=$true;Register;$journal=$OwnedState.journal;$journal.state='installed';Durable $journalPath $journal;SaveOutcome 'idempotent' $null;$script:exitCode=0;return
    }
    NoCliUse;$parent=$null;$apply=$null;$updateJob=$null
    try{
        $running=@(Helpers -RejectForeign);$parent=if($running.Count){Get-Process -Id $running[0].ProcessId -ErrorAction Stop}else{StartHelper '--background --after-update'};if(-not(Ready $parent)){AppFail '原健康助手未能就绪，未开始升级。'}
        $updateJob=Join-Path (Join-Path $data 'updates') ($PID.ToString()+'-'+[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds());[void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($updateJob));[void][IO.Directory]::CreateDirectory($updateJob)
        CopyDurable $PayloadZip (Join-Path $updateJob 'package.zip');CopyDurable $UpdateEnginePath (Join-Path $updateJob 'installer.ps1');Durable (Join-Path $updateJob 'manifest.json') @{schema=1;product='CodexProxyTray';version=$Version;notes='Windows 安装器提供的已校验更新包。'}
        Durable (Join-Path $updateJob 'plan.json') @{schema=1;product='CodexProxyTray';version=$Version;zip_sha256=$ZipSha256;manifest_sha256=(Hash (Join-Path $updateJob 'manifest.json'));executable=$exe;data_root=$data;parent_pid=$parent.Id;parent_created=$parent.StartTime.ToUniversalTime().ToFileTimeUtc().ToString();launch_background=[bool]$Silent}
        ReleaseCodeLock;Event 'preparing-update';$prepare=StartUpdate 'Prepare' $updateJob;try{if(-not $prepare.WaitForExit(90000) -or $prepare.ExitCode -ne 0){AppFail '更新准备未完成，原版本和失败记录已保留。'}}finally{$prepare.Dispose()}
        $verified=Json (Join-Path $updateJob 'verified.json');if($verified.source_commit -cne $SourceCommit){AppFail '更新包来源与安装器不一致，未交接执行。'}
        $apply=StartUpdate 'Apply' $updateJob;$end=[DateTime]::UtcNow.AddSeconds(20);$handoff=Join-Path $updateJob 'handoff-ready.json'
        while(-not(Test-Path -LiteralPath $handoff)){if($apply.HasExited -or [DateTime]::UtcNow -ge $end){AppFail '更新执行器未完成交接，原助手保持运行。'};Start-Sleep -Milliseconds 100}
        $receipt=Json $handoff;if($receipt.schema -ne 1 -or $receipt.pid -ne $apply.Id -or $receipt.state -cne 'waiting-parent'){AppFail '更新交接回执身份无效。'}
        Event 'applying-update';StopRootHelper;if(-not $apply.WaitForExit(120000)){AppFail '更新执行器仍在工作，请保留目录并等待，不要重复运行。'}
        $outcome=Json (Join-Path $updateJob 'result.json');TakeCodeLock
        if($outcome.phase -ceq 'installed_cleanup_pending'){$script:healthy=$true;SaveOutcome 'installed_cleanup_pending' $outcome.error;$script:exitCode=3;return}
        if($outcome.phase -cne 'installed' -or $apply.ExitCode -ne 0){SaveOutcome $(if($outcome.phase -ceq 'rolled_back'){'rolled_back'}else{'error'}) $outcome.error;$script:exitCode=2;return}
        $state=Owned;if($state.active.current.version -cne $Version -or $state.active.current.source_commit -cne $SourceCommit){AppFail '升级结果未对应请求的源码版本。'};$script:healthy=$true;SaveOutcome 'installed' $null;$script:exitCode=0
    }finally{if($parent){$parent.Dispose()};if($apply){$apply.Dispose()}}
}
function RemoveSlot([string]$Path,$Files){if(-not(Test-Path -LiteralPath $Path)){return};Slot $Path $Files -Partial;NoCodeUse @($Path);foreach($file in @($Files)){if(Test-Path -LiteralPath (Join-Path $Path $file.name)){Remove-Item -LiteralPath (Join-Path $Path $file.name) -Force}};[IO.Directory]::Delete($Path)}
function Uninstall {
    $state=Owned -Removing;$script:Version=$state.active.current.version;NoCliUse;StopRootHelper;NoCodeUse @($target,$previous)
    $script:removalStarted=$true;$journal=$state.journal
    if(-not $state.removing){Event 'unregistering';$journal=@{schema=1;product='CodexProxyTray.Installation';install_id=$state.marker.install_id;state='unregister-pending';current=$target;previous=$previous;marker_sha256=(Hash (Join-Path $installRoot 'installation.json'));active_sha256=(Hash $activePath);recovery=$applicationJob};Durable $journalPath $journal;RunOwned $exe @('--unregister-installation') 20;$journal.state='unregistered';Durable $journalPath $journal}
    Event 'removing-owned-code';RemoveSlot $target $state.active.current.files;if(Test-Path -LiteralPath $previous){RemoveSlot $previous $state.active.previous.files}
    if((Hash (Join-Path $installRoot 'installation.json')) -cne $journal.marker_sha256 -or (Hash $activePath) -cne $journal.active_sha256 -or (Hash $uninstaller) -cne $state.marker.uninstaller_sha256){AppFail '卸载期间归属回执发生变化，剩余材料已保留。'}
    foreach($name in @('installation.json','active-version.json','install-journal.json')){CopyDurable (Join-Path $installRoot $name) (Join-Path $applicationJob ('removed-'+$name))}
    Remove-Item -LiteralPath $uninstaller -Force
    ReleaseCodeLock;$lockPath=Join-Path $installRoot '.current-update.lock';if(Test-Path -LiteralPath $lockPath){Plain $lockPath;if((Get-Item -LiteralPath $lockPath).Length -ne 0){AppFail '安装锁内容已变化，卸载收尾停止。'};Remove-Item -LiteralPath $lockPath}
    foreach($name in @('active-version.json','install-journal.json','installation.json')){Remove-Item -LiteralPath (Join-Path $installRoot $name) -Force};[IO.Directory]::Delete($installRoot)
    SaveOutcome 'uninstalled' $null;$script:exitCode=0
}
try {
    if([IntPtr]::Size -ne 8){AppFail 'The installer backend requires native x64 PowerShell.'}
    if(-not $UpdateEnginePath){$UpdateEnginePath=Join-Path $PSScriptRoot 'install_update.ps1'};$UpdateEnginePath=Full $UpdateEnginePath
    . $UpdateEnginePath -Phase Library
    $bootstrapPhase='parse_plan'
    if($PlanPath){$values=InstallPlan (Full $PlanPath);foreach($name in @('Action','InstallRoot','PayloadZip','ZipSha256','Version','SourceCommit','UninstallerPath','ResultPath')){Set-Variable -Name $name -Value ([string]$values[$name]) -Scope Script};$Silent=$values.Silent -ceq '1'}
    $bootstrapPhase='resolve_local_appdata'
    if(-not $env:LOCALAPPDATA -or -not [IO.Path]::IsPathRooted($env:LOCALAPPDATA)){AppFail '当前用户 LOCALAPPDATA 无效。'}
    $localBase=Full $env:LOCALAPPDATA;if($localBase.StartsWith('\\') -or $localBase.StartsWith('\\?\')){AppFail '安装仅支持本机用户目录。'};Plain $localBase
    $defaultRoot=Join-Path $localBase 'Programs\CodexProxyTray';$installRoot=if($InstallRoot){Full $InstallRoot}else{Full $defaultRoot};if($installRoot -ine (Full $defaultRoot)){AppFail '安装根目录必须是当前用户 Programs\CodexProxyTray，未接受其他目录。'}
    $data=Join-Path $localBase 'CodexProxyTray';if((Within $data $installRoot) -or (Within $installRoot $data) -or $data -ieq $installRoot){AppFail '代码目录与用户数据目录不得重叠。'}
    $bootstrapPhase='create_data_root';PlainParent $data;[void][IO.Directory]::CreateDirectory($data);Plain $data;$ownerSid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $outerLock=[IO.File]::Open((Join-Path $data 'application-install.lock'),'OpenOrCreate','ReadWrite','None')
    $jobs=Join-Path $data 'installations';PlainParent $jobs;[void][IO.Directory]::CreateDirectory($jobs);$applicationJob=Join-Path $jobs ($PID.ToString()+'-'+[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds());[void][IO.Directory]::CreateDirectory($applicationJob);$bootstrapPhase='preflight';Event 'preflight'
    $target=Join-Path $installRoot 'current';$previous=Join-Path $installRoot 'previous';$exe=Join-Path $target 'CodexProxyTray.exe';$uninstaller=Join-Path $installRoot 'uninstall.exe';$activePath=Join-Path $installRoot 'active-version.json';$journalPath=Join-Path $installRoot 'install-journal.json';$plan=@{version=$Version;launch_background=[bool]$Silent}
    if($Action -ceq 'Install'){
        [void](Helpers -RejectForeign);if($Version -cnotmatch '^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$' -or $SourceCommit -cnotmatch '^[a-f0-9]{40}$' -or $ZipSha256 -cnotmatch '^[a-f0-9]{64}$'){AppFail '安装器版本或来源参数无效。'}
        $PayloadZip=Full $PayloadZip;$UninstallerPath=Full $UninstallerPath;Plain $PayloadZip;Plain $UninstallerPath;if((Get-Item -LiteralPath $PayloadZip).Length -gt 67108864 -or (Hash $PayloadZip) -cne $ZipSha256 -or (Get-Item -LiteralPath $UninstallerPath).Length -gt 33554432){AppFail '安装器载荷校验失败。'}
        $rootInitiallyEmpty=(-not(Test-Path -LiteralPath $installRoot)) -or @(Get-ChildItem -LiteralPath $installRoot -Force).Count -eq 0
        if(-not $rootInitiallyEmpty){Plain $installRoot;if(Test-Path -LiteralPath (Join-Path $installRoot 'installation.json')){[void](Owned)}elseif(Test-Path -LiteralPath $journalPath){[void](InitialRecord)}else{AppFail '目标目录非空且没有本产品归属记录，未写入任何安装文件。'}}
        PlainParent $installRoot;[void][IO.Directory]::CreateDirectory($installRoot);Plain $installRoot;TakeCodeLock
        if(Test-Path -LiteralPath (Join-Path $installRoot 'installation.json')){$owned=Owned;Upgrade $owned}else{Initialise}
    }else{if(-not(Test-Path -LiteralPath $installRoot -PathType Container)){AppFail '安装归属记录缺失，未执行卸载。'};Plain $installRoot;[void](Owned -Removing);TakeCodeLock;Uninstall}
}catch {
    $message=$_.Exception.Message;if($message.Length -gt 2000){$message=$message.Substring(0,2000)};$phase=if($healthy){'installed_cleanup_pending'}elseif($removalStarted){'uninstall_cleanup_pending'}else{'error'};$exitCode=if($phase -like '*cleanup_pending'){3}else{2}
    try{if($applicationJob){SaveOutcome $phase $message}}catch{[Console]::Error.WriteLine('安装结果保存未完成，原恢复材料已保留。')};[Console]::Error.WriteLine('CodexProxyTray.Installation stage='+$bootstrapPhase+'; '+$message)
}finally{ReleaseCodeLock;if($outerLock){$outerLock.Dispose()}}
exit $exitCode
