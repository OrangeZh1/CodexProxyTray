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
    [ValidateSet('0','1')][string]$DesktopShortcut='0',
    [switch]$Silent
)
$ErrorActionPreference='Stop'
$exitCode=2;$outerLock=$null;$codeLock=$null;$applicationJob=$null;$applicationResult=$null;$healthy=$false;$removalStarted=$false;$bootstrapPhase='load_engine';$failureLocation=$null
function AppFail([string]$Text){throw $Text}
function Full([string]$Path){if(-not $Path -or $Path.Contains([char]0) -or $Path.Contains('"')){AppFail '安装路径无效。'};[IO.Path]::GetFullPath($Path).TrimEnd('\')}
function Within([string]$Path,[string]$Directory){$Path.StartsWith($Directory.TrimEnd('\')+'\',[StringComparison]::OrdinalIgnoreCase)}
function PlainParent([string]$Path){$cursor=Full $Path;while(-not(Test-Path -LiteralPath $cursor)){$parent=[IO.Path]::GetDirectoryName($cursor);if(-not $parent -or $parent -eq $cursor){AppFail '安装目录的上级路径无效。'};$cursor=$parent};Plain $cursor}
function InstallDirectory([string]$Path){
    $value=$Path.Replace('/','\');if($value -cnotmatch '^[A-Za-z]:\\' -or $value -match '[\x00-\x1f\x7f"<>|?*]' -or $value.Substring(2).Contains(':')){AppFail '安装目录必须是本机盘符下的绝对路径，不能使用网络、设备路径或控制字符。'}
    foreach($part in $value.Substring(3).TrimEnd('\').Split('\')){if(-not $part -or $part -in @('.','..') -or $part.EndsWith('.') -or $part.EndsWith(' ') -or $part -match '^(CON|PRN|AUX|NUL|COM[1-9¹²³]|LPT[1-9¹²³])(?:\.|$)'){AppFail '安装目录包含无效路径片段。'}}
    $resolved=Full $value;$drive=[IO.DriveInfo]::new([IO.Path]::GetPathRoot($resolved));if(-not $drive.IsReady -or $drive.DriveType -notin @([IO.DriveType]::Fixed,[IO.DriveType]::Removable,[IO.DriveType]::Ram) -or $resolved.Length -le 3){AppFail '安装目录必须位于可用本机磁盘且不能是磁盘根目录。'}
    if((Within $data $resolved) -or (Within $resolved $data) -or $data -ieq $resolved){AppFail '代码目录与用户数据目录不得重叠。'};PlainParent $resolved;$resolved
}
function Event([string]$State){if(-not $applicationJob){return};$record=@{at=[DateTime]::UtcNow.ToString('o');action=$Action;phase=$State};[IO.File]::AppendAllText((Join-Path $applicationJob 'events.jsonl'),(($record|ConvertTo-Json -Compress)+"`r`n"),[Text.UTF8Encoding]::new($false));Durable (Join-Path $applicationJob 'phase.json') $record}
function SaveOutcome([string]$State,[string]$Message){
    $status=if($State -in @('installed','idempotent','uninstalled')){'passed'}elseif($State -like '*cleanup_pending'){'partial'}else{'failed'}
    $value=@{schema=1;product='CodexProxyTray.Installation';action=$Action;status=$status;phase=$State;version=$Version;root=$installRoot;job=$applicationJob;error=$Message;completed_at=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds();optional_service='unchanged'}
    if($failureLocation){$value.error_location=$failureLocation}
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
    $keys=@('Action','InstallRoot','PayloadZip','ZipSha256','Version','SourceCommit','UninstallerPath','UpdateEnginePath','ResultPath','Silent');if($values.Count -notin @(10,11) -or @($keys|Where-Object {-not $values.ContainsKey($_)}).Count -or @($values.Keys|Where-Object {$_ -cnotin ($keys+@('DesktopShortcut'))}).Count -or $values.Action -cnotin @('Install','Uninstall') -or $values.Silent -cnotin @('0','1') -or ($values.ContainsKey('DesktopShortcut') -and $values.DesktopShortcut -cnotin @('0','1')) -or (Full $values.UpdateEnginePath) -ine $UpdateEnginePath){AppFail '安装调用计划与当前处理程序不匹配。'};if(-not $values.ContainsKey('DesktopShortcut')){$values.DesktopShortcut='0'};$values
}
function Helpers([switch]$RejectForeign){
    $matches=@();foreach($process in @(Get-CimInstance Win32_Process -Filter "name='CodexProxyTray.exe'")){if(-not $process.ExecutablePath){AppFail '某个助手进程的路径无法核验，未停止或替换任何程序。'};if($process.ExecutablePath -ieq $exe){$matches+=,$process}elseif($RejectForeign){AppFail '另一目录的助手仍在运行，请先从该助手正常退出；原便携目录不会自动移动或删除。'}};if($matches.Count -gt 1){AppFail '当前安装目录存在多个助手进程，请先正常退出。'};@($matches)
}
function NoCliUse {foreach($process in @(Get-CimInstance Win32_Process -Filter "name='codex-proxy-cli.exe'")){if(-not $process.ExecutablePath){AppFail '无法核验某个 CLI 入口的路径，安装操作已停止。'};if((Within $process.ExecutablePath $target) -or (Within $process.ExecutablePath $previous)){AppFail '当前安装目录的 Codex CLI 仍在使用，请先结束对应 CLI；其他 ChatGPT 和 CLI 进程不会被停止。'}}}
function StopRootHelper([Diagnostics.Process]$ExpectedProcess=$null) {
    $disposeProcess=$null -eq $ExpectedProcess;$process=$ExpectedProcess
    if(-not $process){$owned=@(Helpers);if(-not $owned.Count){return};$process=Get-Process -Id $owned[0].ProcessId -ErrorAction Stop}
    try{[void]$process.Handle;if($process.HasExited){return};if($process.Path -ine $exe){AppFail '助手进程身份已变化，未发送退出请求。'};RunOwned $exe @('--quit') 15;if(-not $process.WaitForExit(25000)){AppFail '当前助手未正常退出，代码和资料均已保留。'}}finally{if($disposeProcess){$process.Dispose()}}
}
function TakeCodeLock {$script:codeLock=[IO.File]::Open((Join-Path $installRoot '.current-update.lock'),'OpenOrCreate','ReadWrite','None')}
function ReleaseCodeLock {if($codeLock){$codeLock.Dispose();$script:codeLock=$null}}
function Marker {
    $value=Json (Join-Path $installRoot 'installation.json');$keys=@('schema','product','install_id','owner_sid','root','uninstaller_sha256');$names=@($value.PSObject.Properties.Name)
    if($names.Count -ne $keys.Count -or @($names|Where-Object {$_ -cnotin $keys}).Count -or $value.schema -ne 1 -or $value.product -cne 'CodexProxyTray.Installation' -or $value.install_id -cnotmatch '^[a-f0-9]{32}$' -or $value.owner_sid -cne $ownerSid -or (Full $value.root) -ine $installRoot -or $value.uninstaller_sha256 -cnotmatch '^[a-f0-9]{64}$' -or (Hash $uninstaller) -cne $value.uninstaller_sha256){AppFail '安装归属记录不匹配，原目录已保留。'};$value
}
function ReadPreferences($Marker){
    $folder=Join-Path $data 'installation-preferences-v1';PlainParent $folder;$path=Join-Path $folder ($Marker.install_id+'.json');if(-not(Test-Path -LiteralPath $path)){return $null};if((Get-Item -LiteralPath $path).Length -gt 16384){AppFail '安装偏好超过读取上限，原文件已保留。'}
    $value=Json $path;$keys=@('schema','product','install_id','root','desktop_shortcut');$names=@($value.PSObject.Properties.Name)
    if($names.Count -ne 5 -or @($names|Where-Object {$_ -cnotin $keys}).Count -or -not ($value.schema -is [int] -or $value.schema -is [long]) -or $value.schema -ne 1 -or $value.product -cne 'CodexProxyTray.InstallationPreferences' -or $value.install_id -cne $Marker.install_id -or $value.root -isnot [string] -or (Full $value.root) -ine $installRoot -or $value.desktop_shortcut -isnot [bool]){AppFail '安装偏好归属或格式无效，原文件已保留。'};$value
}
function WritePreferences($Marker){
    [void](ReadPreferences $Marker);$folder=Join-Path $data 'installation-preferences-v1';PlainParent $folder;[void][IO.Directory]::CreateDirectory($folder);Plain $folder
    Durable (Join-Path $folder ($Marker.install_id+'.json')) @{schema=1;product='CodexProxyTray.InstallationPreferences';install_id=$Marker.install_id;root=$installRoot;desktop_shortcut=($DesktopShortcut -ceq '1')}
}
function RegisteredRoot {
    $key='HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexProxyTray';if(-not(Test-Path -LiteralPath $key)){return $null};$entry=Get-ItemProperty -LiteralPath $key
    if($entry.CodexProxyTrayOwner -isnot [string] -or $entry.CodexProxyTrayOwner.Length -gt 16384 -or $entry.InstallLocation -isnot [string]){AppFail '现有产品注册项缺少可验证归属，未创建另一份安装。'}
    $receipt=$entry.CodexProxyTrayOwner|ConvertFrom-Json;$keys=@('schema','product','install_id','owner_sid','root','uninstaller_sha256');$names=@($receipt.PSObject.Properties.Name)
    if($names.Count -ne 6 -or @($names|Where-Object {$_ -cnotin $keys}).Count -or $receipt.schema -ne 1 -or $receipt.product -cne 'CodexProxyTray.Installation' -or $receipt.owner_sid -cne $ownerSid -or $receipt.root -isnot [string]){AppFail '现有产品注册归属无效，未创建另一份安装。'}
    # Dynamic function scope lets the unchanged Owned checks inspect the registered root.
    $installRoot=InstallDirectory $receipt.root;$target=Join-Path $installRoot 'current';$previous=Join-Path $installRoot 'previous';$exe=Join-Path $target 'CodexProxyTray.exe';$uninstaller=Join-Path $installRoot 'uninstall.exe';$activePath=Join-Path $installRoot 'active-version.json';$journalPath=Join-Path $installRoot 'install-journal.json'
    if((Full $entry.InstallLocation) -ine $installRoot -or $entry.DisplayName -cne 'CodexProxyTray'){AppFail '产品注册位置与归属记录不一致，原安装已保留。'};$owned=Owned
    foreach($name in $keys){if([string]$receipt.$name -cne [string]$owned.marker.$name){AppFail '产品注册归属与实际文件不一致，原安装已保留。'}};$installRoot
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
function RecoveryKeys($Value,[string[]]$Keys){if('schema' -cin $Keys -and (-not ($Value.schema -is [int] -or $Value.schema -is [long]) -or $Value.schema -ne 1)){AppFail '切换前恢复记录格式版本无效。'};$names=if($Value -is [Collections.IDictionary]){@($Value.Keys)}else{@($Value.PSObject.Properties.Name)};if($names.Count -ne $Keys.Count -or @($names|Where-Object {$_ -cnotin $Keys}).Count){AppFail '切换前恢复记录包含未知字段或缺少绑定。'}}
function RecoveryPath($Value){if($Value -isnot [string] -or -not [IO.Path]::IsPathRooted($Value)){AppFail '切换前恢复路径必须是绝对路径。'};Full $Value}
function RecoveryFiles($Files,[switch]$Empty){
    if($Empty -and -not @($Files).Count){return ''};if(@($Files).Count -ne $payload.Count){AppFail '切换前恢复文件清单不完整。'};$seen=@{}
    foreach($file in @($Files)){RecoveryKeys $file @('name','sha256');if($file.name -isnot [string] -or $file.sha256 -isnot [string] -or $file.name -cnotin $payload -or $file.sha256 -cnotmatch '^[a-f0-9]{64}$' -or $seen.ContainsKey([string]$file.name)){AppFail '切换前恢复文件身份无效。'};$seen[[string]$file.name]=$file.sha256}
    (@($seen.Keys|Sort-Object|ForEach-Object {$_+':'+$seen[$_]}) -join '|')
}
function RecoveryArchive([string]$Path,[string]$ExpectedVersion,[string]$ExpectedSource){
    Plain $Path;if((Get-Item -LiteralPath $Path).Length -gt 67108864){AppFail '恢复更新包超过读取上限。'};$zip=[IO.Compression.ZipFile]::OpenRead($Path);$files=@();$texts=@{};$total=0L;$seen=@{}
    try{if($zip.Entries.Count -ne $payload.Count){AppFail '恢复更新包文件数量无效。'};foreach($entry in $zip.Entries){
        if($entry.FullName -cnotin $payload -or $seen.ContainsKey($entry.FullName) -or $entry.Length -gt 134217728 -or (($entry.ExternalAttributes -shr 16) -band 0xF000) -eq 0xA000){AppFail '恢复更新包含未知路径、重复项或链接。'};$seen[$entry.FullName]=$true;$entryStream=$entry.Open();$digest=[Security.Cryptography.SHA256]::Create();$text=if($entry.FullName -cin @('package-receipt.json','SHA256SUMS.txt')){[IO.MemoryStream]::new()}else{$null};$written=0L
        try{$buffer=[byte[]]::new(65536);while(($count=$entryStream.Read($buffer,0,$buffer.Length)) -gt 0){$written+=$count;$total+=$count;if($written -gt $entry.Length -or $total -gt 134217728 -or ($text -and $written -gt 262144)){AppFail '恢复更新包内容超过声明或读取上限。'};[void]$digest.TransformBlock($buffer,0,$count,$buffer,0);if($text){$text.Write($buffer,0,$count)}};if($written -ne $entry.Length){AppFail '恢复更新包条目不完整。'};[void]$digest.TransformFinalBlock([byte[]]::new(0),0,0);$files+=@{name=$entry.FullName;sha256=([BitConverter]::ToString($digest.Hash)).Replace('-','').ToLowerInvariant()};if($text){$texts[$entry.FullName]=[Text.UTF8Encoding]::new($false,$true).GetString($text.ToArray())}}
        finally{if($text){$text.Dispose()};$digest.Dispose();$entryStream.Dispose()}
    }}finally{$zip.Dispose()}
    [void](RecoveryFiles $files);$receipt=$texts['package-receipt.json']|ConvertFrom-Json
    if($receipt.schema -ne 1 -or $receipt.product -cne 'CodexProxyTray' -or $receipt.version -cne $ExpectedVersion -or $receipt.source_commit -cne $ExpectedSource){AppFail '恢复更新包来源与原校验记录不一致。'}
    $core=@('CodexProxyTray.exe','codex-proxy-cli.exe','WebView2Loader.dll','LICENSE','THIRD_PARTY_NOTICES.md','flag-icons-license.txt');if(@($receipt.files).Count -ne 6 -or @(Compare-Object ($core|Sort-Object) (@($receipt.files.name)|Sort-Object)).Count){AppFail '恢复更新包核心清单无效。'}
    foreach($file in @($receipt.files)){if(@($files|Where-Object {$_.name -ceq $file.name -and $_.sha256 -ceq $file.sha256}).Count -ne 1){AppFail '恢复更新包核心摘要不一致。'}}
    $sums=@();foreach($line in ($texts['SHA256SUMS.txt'] -split '\r?\n')){if(-not $line){continue};if($line -cnotmatch '^([a-f0-9]{64})  ([A-Za-z0-9_.-]+)$' -or $Matches[2] -ceq 'SHA256SUMS.txt') {AppFail '恢复包校验清单无效。'};$sums+=@{name=$Matches[2];sha256=$Matches[1]}};$sums+=@($files|Where-Object name -CEQ 'SHA256SUMS.txt');if((RecoveryFiles $sums) -cne (RecoveryFiles $files)){AppFail '恢复包逐文件校验清单不一致。'}
    [pscustomobject]@{files=$files;receipt=$receipt}
}
function PreSwitchRecovery($Marker,$Active,$Journal){
    RecoveryKeys $Marker @('schema','product','install_id','owner_sid','root','uninstaller_sha256')
    $key='HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexProxyTray';if(-not(Test-Path -LiteralPath $key)){AppFail '切换前恢复缺少本用户产品注册归属。'};$registration=Get-ItemProperty -LiteralPath $key
    if($registration.CodexProxyTrayOwner -isnot [string] -or $registration.CodexProxyTrayOwner.Length -gt 16384 -or $registration.DisplayName -cne 'CodexProxyTray' -or (RecoveryPath $registration.InstallLocation) -ine $installRoot -or $registration.UninstallString -cne ('"'+$uninstaller+'" --uninstall')){AppFail '切换前恢复的 Windows 产品注册项不匹配。'};$registeredOwner=$registration.CodexProxyTrayOwner|ConvertFrom-Json;RecoveryKeys $registeredOwner @('schema','product','install_id','owner_sid','root','uninstaller_sha256');foreach($name in @('schema','product','install_id','owner_sid','root','uninstaller_sha256')){if([string]$registeredOwner.$name -cne [string]$Marker.$name){AppFail '切换前恢复的注册归属与原安装记录不一致。'}}
    RecoveryKeys $Journal @('schema','state','current','previous','source_commit','recovery','candidate','obsolete')
    if($Journal.schema -ne 1 -or $Journal.state -cne 'switching' -or (RecoveryPath $Journal.current) -ine $target -or (RecoveryPath $Journal.previous) -ine $previous -or $Journal.source_commit -cnotmatch '^[a-f0-9]{40}$'){AppFail '只支持原代码槽位完全未移动的切换前恢复。'}
    $job=RecoveryPath $Journal.recovery;$id=[IO.Path]::GetFileName($job);if($id -cnotmatch '^\d+-\d+$' -or [IO.Path]::GetDirectoryName($job) -ine (Join-Path $data 'updates')){AppFail '遗留更新任务目录不属于本机固定恢复范围。'};Plain $job
    $retired=Join-Path $installRoot ('.candidate-update-'+$id);$obsoletePath=Join-Path $installRoot ('.obsolete-update-'+$id)
    if((RecoveryPath $Journal.candidate) -ine $retired -or (RecoveryPath $Journal.obsolete) -ine $obsoletePath -or (Test-Path -LiteralPath $obsoletePath)){AppFail '遗留更新已移动槽位或暂存路径不匹配，未自动恢复。'};RootLayout $Journal $retired
    $allowed=@('package.zip','installer.ps1','manifest.json','release-sums.txt','plan.json','verified.json','pre-active-version.json','pre-install-journal.json','handoff-ready.json','config.json','events-v1.json','result.json','cancelled.json','recovery-switching-journal.json','candidate-retirement-v1.json')
    foreach($item in @(Get-ChildItem -LiteralPath $job -Force)){Plain $item.FullName;if($item.PSIsContainer -or $item.Name -cnotin $allowed){AppFail '遗留更新任务含未知材料，原文件已保留。'}}
    $oldPlan=Json (Join-Path $job 'plan.json');$planKeys=@('schema','product','version','zip_sha256','manifest_sha256','executable','data_root','parent_pid','parent_created');if('launch_background' -cin @($oldPlan.PSObject.Properties.Name)){$planKeys+='launch_background';if($oldPlan.launch_background -isnot [bool]){AppFail '原启动偏好格式无效。'}};RecoveryKeys $oldPlan $planKeys
    if($oldPlan.schema -ne 1 -or $oldPlan.product -cne 'CodexProxyTray' -or $oldPlan.version -cnotmatch '^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$' -or $oldPlan.zip_sha256 -cnotmatch '^[a-f0-9]{64}$' -or $oldPlan.manifest_sha256 -cnotmatch '^[a-f0-9]{64}$' -or (RecoveryPath $oldPlan.executable) -ine $exe -or (RecoveryPath $oldPlan.data_root) -ine $data -or $oldPlan.parent_pid -le 0 -or $oldPlan.parent_created -cnotmatch '^[1-9]\d{0,19}$'){AppFail '遗留更新计划与本安装不匹配。'}
    if((Hash (Join-Path $job 'package.zip')) -cne $oldPlan.zip_sha256 -or (Hash (Join-Path $job 'manifest.json')) -cne $oldPlan.manifest_sha256){AppFail '遗留更新包或发布清单摘要发生变化。'};$oldManifest=Json (Join-Path $job 'manifest.json');if($oldManifest.schema -ne 1 -or $oldManifest.product -cne 'CodexProxyTray' -or $oldManifest.version -cne $oldPlan.version){AppFail '遗留更新发布清单不匹配。'}
    $proof=Json (Join-Path $job 'verified.json');RecoveryKeys $proof @('schema','before','after','old_previous','source_commit','old_active','target','previous','candidate','obsolete')
    if($proof.schema -ne 1 -or $proof.source_commit -cne $Journal.source_commit -or (RecoveryPath $proof.target) -ine $target -or (RecoveryPath $proof.previous) -ine $previous -or (RecoveryPath $proof.candidate) -ine $retired -or (RecoveryPath $proof.obsolete) -ine $obsoletePath){AppFail '遗留更新校验记录的路径或来源不匹配。'}
    $preActive=Join-Path $job 'pre-active-version.json';$preJournal=Join-Path $job 'pre-install-journal.json';$activeHash=Hash $activePath;$preJournalHash=Hash $preJournal
    if((Hash $preActive) -cne $activeHash -or ($proof.old_active|ConvertTo-Json -Depth 24 -Compress) -cne ($Active|ConvertTo-Json -Depth 24 -Compress) -or (RecoveryFiles $proof.before) -cne (RecoveryFiles $Active.current.files)){AppFail '原健康活动版本或切换前备份不一致。'};Slot $target $proof.before
    if(@($proof.old_previous).Count){if((RecoveryPath $Active.previous.path) -ine $previous -or (RecoveryFiles $proof.old_previous) -cne (RecoveryFiles $Active.previous.files)){AppFail '原回退槽位校验记录不匹配。'};Slot $previous $proof.old_previous}elseif((Test-Path -LiteralPath $previous) -or @($Active.previous.files).Count){AppFail '原空回退槽位已发生变化。'}
    $oldJournal=Json $preJournal;if($oldJournal.schema -ne 1 -or $oldJournal.state -cnotin @('installed','installed-partial','active','complete')){AppFail '切换前安装事务不是可确认的健康终态。'}
    if('product' -cin @($oldJournal.PSObject.Properties.Name)){RecoveryKeys $oldJournal @('schema','product','operation','install_id','owner_sid','root','version','source_commit','zip_sha256','uninstaller_sha256','state','candidate','files','recovery','at');if($oldJournal.product -cne 'CodexProxyTray.Installation' -or $oldJournal.install_id -cne $Marker.install_id -or $oldJournal.owner_sid -cne $ownerSid -or (RecoveryPath $oldJournal.root) -ine $installRoot -or $oldJournal.version -cne $Active.current.version -or $oldJournal.source_commit -cne $Active.current.source_commit -or $oldJournal.uninstaller_sha256 -cne $Marker.uninstaller_sha256 -or (RecoveryFiles $oldJournal.files) -cne (RecoveryFiles $proof.before)){AppFail '切换前安装事务归属不匹配。'}}else{RecoveryKeys $oldJournal @('schema','state','current','previous','source_commit','recovery','candidate','obsolete');if((RecoveryPath $oldJournal.current) -ine $target -or (RecoveryPath $oldJournal.previous) -ine $previous -or $oldJournal.source_commit -cne $Active.current.source_commit -or $oldJournal.candidate -or $oldJournal.obsolete){AppFail '切换前更新事务未结束或归属不匹配。'}}
    $outcome=Json (Join-Path $job 'result.json');RecoveryKeys $outcome @('schema','completed_at','busy','phase','version','error','at','recovery');if($outcome.schema -ne 1 -or $outcome.phase -cne 'error' -or $outcome.busy -isnot [bool] -or $outcome.busy -or $outcome.version -cne $oldPlan.version -or (RecoveryPath $outcome.recovery) -ine $job -or $outcome.completed_at -le 0 -or $outcome.error -isnot [string] -or -not $outcome.error){AppFail '遗留更新缺少版本对应的失败终态。'}
    $archive=RecoveryArchive (Join-Path $job 'package.zip') $oldPlan.version $proof.source_commit;if((RecoveryFiles $archive.files) -cne (RecoveryFiles $proof.after)){AppFail '遗留更新包与候选文件摘要不一致。'}
    $evidence=Join-Path $job 'recovery-switching-journal.json';$receiptPath=Join-Path $job 'candidate-retirement-v1.json';$switchHash=Hash $journalPath
    $declaration=@{schema=1;product='CodexProxyTray.CandidateRetirement';state='candidate-retired-original-still-active';owner_sid=$ownerSid;install_id=$Marker.install_id;root=$installRoot;job=$job;journal_sha256=$switchHash;active_sha256=$activeHash;pre_journal_sha256=$preJournalHash;plan_sha256=(Hash (Join-Path $job 'plan.json'));verified_sha256=(Hash (Join-Path $job 'verified.json'));zip_sha256=$oldPlan.zip_sha256;retired_candidate=@{path=$retired;files=@($proof.after);recovery_status='not-a-rollback-target'};verified_role='historical-only'}
    if(Test-Path -LiteralPath $evidence){if((Hash $evidence) -cne $switchHash){AppFail '原切换事务证据副本不一致。'}}
    $hasRetirement=Test-Path -LiteralPath $receiptPath
    if($hasRetirement){if((Get-Item -LiteralPath $receiptPath).Length -gt 16384){AppFail '候选退役回执超过上限。'};$existing=Json $receiptPath;RecoveryKeys $existing @($declaration.Keys);foreach($name in @($declaration.Keys|Where-Object {$_ -cne 'retired_candidate'})){if(($name -cne 'schema' -and $existing.$name -isnot [string]) -or [string]$existing.$name -cne [string]$declaration[$name]){AppFail '候选退役回执的归属或摘要不匹配。'}};RecoveryKeys $existing.retired_candidate @('path','files','recovery_status');if((RecoveryPath $existing.retired_candidate.path) -ine $retired -or $existing.retired_candidate.recovery_status -cne 'not-a-rollback-target' -or (RecoveryFiles $existing.retired_candidate.files) -cne (RecoveryFiles $proof.after) -or -not(Test-Path -LiteralPath $evidence)){AppFail '候选退役清单或原切换证据缺失。'};Slot $retired $proof.after -Partial}else{Slot $retired $proof.after}
    NoCodeUse @($retired);[pscustomobject]@{job=$job;candidate=$retired;files=@($proof.after);pre_journal=$preJournal;receipt_path=$receiptPath;evidence=$evidence;declaration=$declaration;retired=[bool]$hasRetirement}
}
function RecoverPreSwitch($State){
    if(-not $codeLock){AppFail '切换前恢复必须持有原代码锁。'};$fresh=Owned;$recovery=$fresh.recovery;if(-not $recovery){AppFail '切换前恢复状态已变化。'};$oldZipLease=$null;$preJournalLease=$null
    try{
    $oldZipLease=[IO.File]::Open((Join-Path $recovery.job 'package.zip'),'Open','Read','Read');$preJournalLease=[IO.File]::Open($recovery.pre_journal,'Open','Read','Read');Event 'recovering-pre-switch'
    if(-not(Test-Path -LiteralPath $recovery.evidence)){CopyDurable $journalPath $recovery.evidence}
    if(-not $recovery.retired){$bytes=[Text.UTF8Encoding]::new($false).GetBytes(($recovery.declaration|ConvertTo-Json -Depth 24));$stream=[IO.File]::Open($recovery.receipt_path,'CreateNew','Write','None');try{$stream.Write($bytes,0,$bytes.Length);$stream.Flush($true)}finally{$stream.Dispose()}}
    $confirmed=Owned;$recovery=$confirmed.recovery;if(-not $recovery.retired){AppFail '候选退役声明未持久确认。'}
    if(Test-Path -LiteralPath $recovery.candidate){Slot $recovery.candidate $recovery.files -Partial;NoCodeUse @($recovery.candidate);foreach($file in @($recovery.files)){$path=Join-Path $recovery.candidate $file.name;if(Test-Path -LiteralPath $path){Plain $path;if((Hash $path) -cne $file.sha256){AppFail '退役候选文件删除前发生变化，原文件已保留。'};Remove-Item -LiteralPath $path -Force}};[IO.Directory]::Delete($recovery.candidate)}
    if((Hash $journalPath) -cne $recovery.declaration.journal_sha256 -or (Hash $activePath) -cne $recovery.declaration.active_sha256){AppFail '清理期间原安装回执发生变化。'};if((Hash $recovery.pre_journal) -cne $recovery.declaration.pre_journal_sha256){AppFail '恢复源安装事务摘要发生变化，未替换原切换记录。'};RestoreConfig $recovery.pre_journal $journalPath
    if((Hash $journalPath) -cne $recovery.declaration.pre_journal_sha256){AppFail '原安装事务未能精确恢复。'};Event 'pre-switch-restored';Owned
    }finally{if($preJournalLease){$preJournalLease.Dispose()};if($oldZipLease){$oldZipLease.Dispose()}}
}
function Owned([switch]$Removing){
    $marker=Marker;$active=Json $activePath;$journal=Json $journalPath;$recovery=$null
    if($journal.schema -ne 1){AppFail '安装事务记录格式版本无效，已保留。'}
    if('product' -cin @($journal.PSObject.Properties.Name)){
        if($journal.product -cne 'CodexProxyTray.Installation' -or $journal.install_id -cne $marker.install_id -or ($journal.root -and (Full $journal.root) -ine $installRoot) -or ($journal.owner_sid -and $journal.owner_sid -cne $ownerSid)){AppFail '安装事务的产品或用户归属不匹配。'}
    }elseif((Full $journal.current) -ine $target -or (Full $journal.previous) -ine $previous){AppFail '更新事务记录的代码路径不匹配。'}
    if($active.schema -ne 1 -or $active.state -cne 'active' -or (Full $active.current.path) -ine $target -or $active.current.version -cnotmatch '^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$' -or $active.readiness.native -cne 'passed' -or $active.readiness.main_rendered -ne $true -or $active.readiness.tray_rendered -ne $true){AppFail '活动版本或健康回执不完整，已保留。'}
    if(-not $Removing -and $journal.state -ceq 'switching'){$recovery=PreSwitchRecovery $marker $active $journal}else{RootLayout $journal}
    $resumeRemoval=($Removing -and $journal.state -ceq 'unregistered' -and $journal.product -ceq 'CodexProxyTray.Installation' -and $journal.install_id -ceq $marker.install_id)
    if(-not $recovery -and $journal.state -cnotin @('installed','installed-partial','active','complete','registration-pending','cleanup-blocked','unregister-pending','unregistered')){AppFail '存在未完成安装事务，请先处理恢复记录。'}
    if($journal.state -ceq 'cleanup-blocked' -and ($journal.candidate -or $journal.obsolete)){AppFail '已有安装收尾仍引用暂存版本，未重新部署。'}
    if(-not $Removing -and $journal.state -cin @('unregister-pending','unregistered')){AppFail '已有卸载尚未收尾，未开始新的安装。'}
    if($journal.state -cin @('unregister-pending','unregistered')){if($journal.marker_sha256 -cnotmatch '^[a-f0-9]{64}$' -or $journal.active_sha256 -cnotmatch '^[a-f0-9]{64}$' -or (Hash (Join-Path $installRoot 'installation.json')) -cne $journal.marker_sha256 -or (Hash $activePath) -cne $journal.active_sha256){AppFail '卸载恢复摘要与当前归属回执不一致，未删除代码。'}}
    Slot $target $active.current.files -Partial:$resumeRemoval
    if(Test-Path -LiteralPath $previous){if((Full $active.previous.path) -ine $previous){AppFail '上一版本的目录归属不明。'};Slot $previous $active.previous.files -Partial:$resumeRemoval}
    if(-not $resumeRemoval){$receipt=Json (Join-Path $target 'package-receipt.json');if($receipt.schema -ne 1 -or $receipt.product -cne 'CodexProxyTray' -or $receipt.version -cne $active.current.version -or $receipt.source_commit -cne $active.current.source_commit -or (Get-Item -LiteralPath $exe).VersionInfo.ProductVersion -cne $active.current.version){AppFail '当前程序来源或版本与回执不一致。'}}
    References $active;if(-not $Removing){[void](ReadPreferences $marker)};[pscustomobject]@{marker=$marker;active=$active;journal=$journal;removing=[bool]$resumeRemoval;recovery=$recovery}
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
    try{WritePreferences $marker;Register;$journal.state='installed';Durable $journalPath $journal;SaveOutcome 'installed' $null;$script:exitCode=0}catch{$journal.state='registration-pending';Durable $journalPath $journal;throw}
}
function Upgrade($OwnedState){
    NoCliUse
    if([version]$Version -lt [version]$OwnedState.active.current.version){AppFail '目标版本低于当前健康版本，未执行降级。'}
    if($Version -ceq $OwnedState.active.current.version){
        if($SourceCommit -cne $OwnedState.active.current.source_commit){AppFail '同版本的编译来源不同，未覆盖已安装程序。'};$script:healthy=$true;WritePreferences $OwnedState.marker;Register;$journal=$OwnedState.journal;$journal.state='installed';Durable $journalPath $journal;SaveOutcome 'idempotent' $null;$script:exitCode=0;return
    }
    NoCliUse;$parent=$null;$apply=$null;$updateJob=$null
    try{
        $running=@(Helpers -RejectForeign);$parent=if($running.Count){Get-Process -Id $running[0].ProcessId -ErrorAction Stop}else{StartHelper '--background --after-update'};[void]$parent.Handle;if($parent.Path -ine $exe -or -not(Ready $parent)){AppFail '原健康助手未能就绪，未开始升级。'}
        # This explicit user preference survives a failed update; code/config rollback is independent.
        WritePreferences $OwnedState.marker
        $updateJob=Join-Path (Join-Path $data 'updates') ($PID.ToString()+'-'+[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds());[void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($updateJob));[void][IO.Directory]::CreateDirectory($updateJob)
        CopyDurable $PayloadZip (Join-Path $updateJob 'package.zip');CopyDurable $UpdateEnginePath (Join-Path $updateJob 'installer.ps1');Durable (Join-Path $updateJob 'manifest.json') @{schema=1;product='CodexProxyTray';version=$Version;notes='Windows 安装器提供的已校验更新包。'}
        # Setup verifies readiness in the background; its Finish-page Run choice opens the window.
        Durable (Join-Path $updateJob 'plan.json') @{schema=1;product='CodexProxyTray';version=$Version;zip_sha256=$ZipSha256;manifest_sha256=(Hash (Join-Path $updateJob 'manifest.json'));executable=$exe;data_root=$data;parent_pid=$parent.Id;parent_created=$parent.StartTime.ToUniversalTime().ToFileTimeUtc().ToString();launch_background=$true}
        ReleaseCodeLock;Event 'preparing-update';$prepare=StartUpdate 'Prepare' $updateJob;try{if(-not $prepare.WaitForExit(90000) -or $prepare.ExitCode -ne 0){AppFail '更新准备未完成，原版本和失败记录已保留。'}}finally{$prepare.Dispose()}
        $verified=Json (Join-Path $updateJob 'verified.json');if($verified.source_commit -cne $SourceCommit){AppFail '更新包来源与安装器不一致，未交接执行。'}
        $apply=StartUpdate 'Apply' $updateJob;$end=[DateTime]::UtcNow.AddSeconds(20);$handoff=Join-Path $updateJob 'handoff-ready.json'
        while(-not(Test-Path -LiteralPath $handoff)){if($apply.HasExited -or [DateTime]::UtcNow -ge $end){AppFail '更新执行器未完成交接，原助手保持运行。'};Start-Sleep -Milliseconds 100}
        $receipt=Json $handoff;if($receipt.schema -ne 1 -or $receipt.pid -ne $apply.Id -or $receipt.state -cne 'waiting-parent'){AppFail '更新交接回执身份无效。'}
        Event 'applying-update';StopRootHelper $parent;if(-not $apply.WaitForExit(120000)){AppFail '更新执行器仍在工作，请保留目录并等待，不要重复运行。'}
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
    if($PlanPath){$values=InstallPlan (Full $PlanPath);foreach($name in @('Action','InstallRoot','PayloadZip','ZipSha256','Version','SourceCommit','UninstallerPath','ResultPath','DesktopShortcut')){Set-Variable -Name $name -Value ([string]$values[$name]) -Scope Script};$Silent=$values.Silent -ceq '1'}
    $bootstrapPhase='resolve_local_appdata'
    if(-not $env:LOCALAPPDATA -or -not [IO.Path]::IsPathRooted($env:LOCALAPPDATA)){AppFail '当前用户 LOCALAPPDATA 无效。'}
    $localBase=Full $env:LOCALAPPDATA;if($localBase.StartsWith('\\') -or $localBase.StartsWith('\\?\')){AppFail '安装仅支持本机用户目录。'};Plain $localBase
    $data=Join-Path $localBase 'CodexProxyTray';$defaultRoot=Join-Path $localBase 'Programs\CodexProxyTray';$installRoot=InstallDirectory $(if($InstallRoot){$InstallRoot}else{$defaultRoot})
    $bootstrapPhase='create_data_root';PlainParent $data;[void][IO.Directory]::CreateDirectory($data);Plain $data;$ownerSid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $outerLock=[IO.File]::Open((Join-Path $data 'application-install.lock'),'OpenOrCreate','ReadWrite','None')
    $jobs=Join-Path $data 'installations';PlainParent $jobs;[void][IO.Directory]::CreateDirectory($jobs);$applicationJob=Join-Path $jobs ($PID.ToString()+'-'+[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds());[void][IO.Directory]::CreateDirectory($applicationJob);$bootstrapPhase='preflight';Event 'preflight'
    $target=Join-Path $installRoot 'current';$previous=Join-Path $installRoot 'previous';$exe=Join-Path $target 'CodexProxyTray.exe';$uninstaller=Join-Path $installRoot 'uninstall.exe';$activePath=Join-Path $installRoot 'active-version.json';$journalPath=Join-Path $installRoot 'install-journal.json';$plan=@{version=$Version;launch_background=[bool]$Silent}
    if($Action -ceq 'Install'){
        $registered=RegisteredRoot;if($registered -and $registered -ine $installRoot){AppFail '已存在经过验证的安装，请使用原安装目录升级；未搬迁或创建第二份安装。'}
        [void](Helpers -RejectForeign);if($Version -cnotmatch '^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$' -or $SourceCommit -cnotmatch '^[a-f0-9]{40}$' -or $ZipSha256 -cnotmatch '^[a-f0-9]{64}$'){AppFail '安装器版本或来源参数无效。'}
        $PayloadZip=Full $PayloadZip;$UninstallerPath=Full $UninstallerPath;Plain $PayloadZip;Plain $UninstallerPath;if((Get-Item -LiteralPath $PayloadZip).Length -gt 67108864 -or (Hash $PayloadZip) -cne $ZipSha256 -or (Get-Item -LiteralPath $UninstallerPath).Length -gt 33554432){AppFail '安装器载荷校验失败。'}
        $rootInitiallyEmpty=(-not(Test-Path -LiteralPath $installRoot)) -or @(Get-ChildItem -LiteralPath $installRoot -Force).Count -eq 0
        if(-not $rootInitiallyEmpty){Plain $installRoot;if(Test-Path -LiteralPath (Join-Path $installRoot 'installation.json')){[void](Owned)}elseif(Test-Path -LiteralPath $journalPath){[void](InitialRecord)}else{AppFail '目标目录非空且没有本产品归属记录，未写入任何安装文件。'}}
        PlainParent $installRoot;[void][IO.Directory]::CreateDirectory($installRoot);Plain $installRoot;TakeCodeLock
        if(Test-Path -LiteralPath (Join-Path $installRoot 'installation.json')){$owned=Owned;if([version]$Version -lt [version]$owned.active.current.version){AppFail '目标版本低于当前健康版本，未执行降级。'};if($owned.recovery){$payloadLease=[IO.File]::Open($PayloadZip,'Open','Read','Read');try{if((Hash $PayloadZip) -cne $ZipSha256){AppFail '恢复前新安装包摘要发生变化。'};[void](RecoveryArchive $PayloadZip $Version $SourceCommit);$owned=RecoverPreSwitch $owned}finally{$payloadLease.Dispose()}};Upgrade $owned}else{Initialise}
    }else{if(-not(Test-Path -LiteralPath $installRoot -PathType Container)){AppFail '安装归属记录缺失，未执行卸载。'};Plain $installRoot;[void](Owned -Removing);TakeCodeLock;Uninstall}
}catch {
    $failureFrames=@();foreach($frame in @($_.ScriptStackTrace -split '\r?\n'|Select-Object -First 6)){if($frame -match '^at ([^,]+), .*: line (\d+)$'){$frameFunction=$Matches[1];$frameLine=[int]$Matches[2];if($frameFunction -cmatch '^(?:[A-Za-z][A-Za-z0-9_-]*|<ScriptBlock>)$'){$failureFrames+=@{function=$frameFunction;line=$frameLine}}}};$failureLocation=@{stage=$bootstrapPhase;line=[int]$_.InvocationInfo.ScriptLineNumber;frames=$failureFrames}
    $message=$_.Exception.Message;if($message.Length -gt 2000){$message=$message.Substring(0,2000)};$outcomePhase=if($healthy){'installed_cleanup_pending'}elseif($removalStarted){'uninstall_cleanup_pending'}else{'error'};$exitCode=if($outcomePhase -like '*cleanup_pending'){3}else{2}
    try{if($applicationJob){SaveOutcome $outcomePhase $message}}catch{[Console]::Error.WriteLine('安装结果保存未完成，原恢复材料已保留。')};[Console]::Error.WriteLine('CodexProxyTray.Installation stage='+$bootstrapPhase+'; '+$message)
}finally{ReleaseCodeLock;if($outerLock){$outerLock.Dispose()}}
exit $exitCode
