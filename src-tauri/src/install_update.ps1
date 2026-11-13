param([ValidateSet('Prepare','Apply','Library')][string]$Phase,[string]$Job)
$ErrorActionPreference='Stop'
# The helper uses only protected Windows modules, independent of a PowerShell 7 parent.
$originalModulePath=$env:PSModulePath
$originalModulesPresent=$null -ne $originalModulePath
if($env:CODEX_PROXY_UPDATER_MODULES_PRESENT -in @('0','1')){$originalModulesPresent=$env:CODEX_PROXY_UPDATER_MODULES_PRESENT -eq '1';$originalModulePath=$env:CODEX_PROXY_UPDATER_MODULES}
$originalPolicyPresent=$env:CODEX_PROXY_UPDATER_POLICY_PRESENT -eq '1'
$originalPolicy=$env:CODEX_PROXY_UPDATER_POLICY
$env:PSModulePath=Join-Path $PSHOME 'Modules'
Add-Type -AssemblyName System.IO.Compression.FileSystem
function Plain([string]$Path){$cursor=[IO.Path]::GetFullPath($Path);while($cursor){$item=Get-Item -LiteralPath $cursor -Force;if($item.Attributes -band [IO.FileAttributes]::ReparsePoint){throw '路径包含重解析点，已停止更新。'};$parent=[IO.Path]::GetDirectoryName($cursor);if($parent -eq $cursor){break};$cursor=$parent}}
function Hash([string]$Path){$algorithm=[Security.Cryptography.SHA256]::Create();$stream=$null;try{$stream=[IO.File]::OpenRead($Path);([BitConverter]::ToString($algorithm.ComputeHash($stream))).Replace('-','').ToLowerInvariant()}finally{if($stream){$stream.Dispose()};$algorithm.Dispose()}}
function Json([string]$Path){Plain $Path;if((Get-Item -LiteralPath $Path).Length -gt 262144){throw '安装记录过大。'};Get-Content -LiteralPath $Path -Raw -Encoding UTF8|ConvertFrom-Json}
function CopyDurable([string]$From,[string]$To){Plain $From;Plain ([IO.Path]::GetDirectoryName($To));$sourceStream=[IO.File]::Open($From,'Open','Read','Read');$output=[IO.File]::Open($To,'CreateNew','Write','None');try{$sourceStream.CopyTo($output);$output.Flush($true)}finally{$output.Dispose();$sourceStream.Dispose()};if((Hash $From) -cne (Hash $To)){throw '恢复副本校验失败。'}}
function RestoreConfig([string]$From,[string]$To){$temporary=$To+'.restore-pending';CopyDurable $From $temporary;if(Test-Path -LiteralPath $To){Plain $To;[IO.File]::Replace($temporary,$To,[NullString]::Value)}else{[IO.File]::Move($temporary,$To)};if((Hash $From) -cne (Hash $To)){throw '配置恢复校验失败。'}}
function Durable([string]$Path,$Value){$temp=$Path+'.pending';$bytes=[Text.UTF8Encoding]::new($false).GetBytes(($Value|ConvertTo-Json -Depth 24));$stream=[IO.File]::Open($temp,'CreateNew','Write','None');try{$stream.Write($bytes,0,$bytes.Length);$stream.Flush($true)}finally{$stream.Dispose()};if(Test-Path -LiteralPath $Path){[IO.File]::Replace($temp,$Path,[NullString]::Value)}else{[IO.File]::Move($temp,$Path)};[void](Json $Path)}
function Inventory([string]$Path){Plain $Path;$items=@(Get-ChildItem -LiteralPath $Path -Force);if(-not $items.Count){throw '安装目录为空。'};foreach($item in $items){if($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -or $item.Name -cnotin $payload){throw '安装目录有未知文件，已保留原目录。'}};@($items|ForEach-Object {@{name=$_.Name;sha256=Hash $_.FullName}})}
function Verify([string]$Path,$Files){$actual=@(Inventory $Path);if(@($Files).Count -ne $actual.Count){throw '安装文件数量发生变化。'}; $seen=@{};foreach($file in $Files){if($seen.ContainsKey([string]$file.name)){throw '安装校验清单含重复文件。'};$seen[[string]$file.name]=$true;if($file.name -cnotin $payload -or @($actual|Where-Object {$_.name -ceq $file.name -and $_.sha256 -ceq $file.sha256}).Count -ne 1){throw '安装文件校验失败。'}}}
function SafeMove([string]$From,[string]$To){Plain $From;Plain ([IO.Path]::GetDirectoryName($To));if((Test-Path -LiteralPath $To) -or [IO.Path]::GetPathRoot($From) -ne [IO.Path]::GetPathRoot($To)){throw '代码版本目录不满足原子切换条件。'};Move-Item -LiteralPath $From -Destination $To}
function NoCodeUse([string[]]$Paths){foreach($process in @(Get-CimInstance Win32_Process -Filter "name='CodexProxyTray.exe' OR name='codex-proxy-cli.exe'")){if(-not $process.ExecutablePath){throw '无法核验某个助手进程的映像归属，旧版本清理暂缓。'};foreach($path in $Paths){if($process.ExecutablePath.StartsWith($path+'\',[StringComparison]::OrdinalIgnoreCase)){throw ('旧版本仍被进程使用，已保留代码。PID '+$process.ProcessId)}}}}
function RemoveCode([string]$Path,$Files){if([IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($Path)) -ne $installRoot -or $Path -notin @($candidate,$obsolete)){throw '清理目录超出本次安装范围。'};Verify $Path $Files;NoCodeUse @($Path,$previous);foreach($file in $Files){Remove-Item -LiteralPath (Join-Path $Path $file.name) -Force};[IO.Directory]::Delete($Path)}
function Result([string]$State,[string]$ErrorText){$value=@{schema=1;completed_at=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds();busy=$false;phase=$State;version=$plan.version;error=$ErrorText;at=[DateTime]::UtcNow.ToString('o');recovery=$Job};Durable (Join-Path $Job 'result.json') $value;Durable (Join-Path $data 'update-install-result.json') $value;if($State -in @('rolled_back','error')){if(-not(Test-Path -LiteralPath (Join-Path $Job 'cancelled.json'))){Durable (Join-Path $Job 'cancelled.json') @{schema=1;state=$State}}};$readyPath=Join-Path $data 'update-download-ready.json';if(Test-Path -LiteralPath $readyPath){$ready=Json $readyPath;if($ready.schema -eq 1 -and $ready.job -ceq $id){Remove-Item -LiteralPath $readyPath}}}

function SameParent($Process){$Process -and $Process.StartTime.ToUniversalTime().ToFileTimeUtc().ToString() -ceq $plan.parent_created -and $Process.MainModule.FileName -ieq $exe}
function OwnProcesses([string]$Image){@(Get-CimInstance Win32_Process -Filter "name='CodexProxyTray.exe'"|Where-Object {$_.ExecutablePath -ieq $Image})}
function WaitGone([int]$Id,[string]$Created,[int]$Seconds){$end=[DateTime]::UtcNow.AddSeconds($Seconds);do{if(Test-Path -LiteralPath (Join-Path $Job 'cancelled.json')){return $false};$process=Get-Process -Id $Id -ErrorAction SilentlyContinue;if(-not $process -or $process.StartTime.ToUniversalTime().ToFileTimeUtc().ToString() -cne $Created){return $true};Start-Sleep -Milliseconds 200}while([DateTime]::UtcNow -lt $end);return $false}
function StartHelper([string]$Arguments='--after-update'){if($Arguments -ceq '--after-update' -and $plan.launch_background -eq $true){$Arguments='--background --after-update'};$start=[Diagnostics.ProcessStartInfo]::new($exe);$start.Arguments=$Arguments;$start.UseShellExecute=$false;$start.CreateNoWindow=$true;$start.WindowStyle='Hidden';$start.WorkingDirectory=$target;if($originalModulesPresent){$start.EnvironmentVariables['PSModulePath']=$originalModulePath}else{$start.EnvironmentVariables.Remove('PSModulePath')};if($originalPolicyPresent){$start.EnvironmentVariables['PSExecutionPolicyPreference']=$originalPolicy}else{$start.EnvironmentVariables.Remove('PSExecutionPolicyPreference')};$start.EnvironmentVariables.Remove('CODEX_PROXY_UPDATER_POLICY');$start.EnvironmentVariables.Remove('CODEX_PROXY_UPDATER_POLICY_PRESENT');$start.EnvironmentVariables.Remove('CODEX_PROXY_UPDATER_MODULES');$start.EnvironmentVariables.Remove('CODEX_PROXY_UPDATER_MODULES_PRESENT');[Diagnostics.Process]::Start($start)}
function Ready($Process){$end=[DateTime]::UtcNow.AddSeconds(40);do{if($Process.HasExited){return $false};$ok=$true;foreach($window in @('main','tray')){try{$ready=Json (Join-Path $data "ui-ready-$window.json");if($ready.helper_pid -ne $Process.Id -or $ready.rendered -ne $true -or $ready.at -lt ([DateTimeOffset]$Process.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()){$ok=$false}}catch{$ok=$false}};if($ok){$errorFile=Join-Path $data 'ui-error.json';if(Test-Path -LiteralPath $errorFile){$errorRecord=Json $errorFile;if($errorRecord.helper_pid -eq $Process.Id){return $false}};return $true};Start-Sleep -Milliseconds 250}while([DateTime]::UtcNow -lt $end);return $false}
function ExpandPayload([string]$ZipPath,[string]$Destination,[string]$ExpectedVersion,[string]$ExpectedSource=''){
    Plain $ZipPath;Plain $Destination;if(@(Get-ChildItem -LiteralPath $Destination -Force).Count){throw '安装暂存目录非空，原内容已保留。'}
    $zip=[IO.Compression.ZipFile]::OpenRead($ZipPath)
    try{if($zip.Entries.Count -ne $payload.Count){throw '安装包文件数量无效。'};$seen=@{};$expanded=0L;$writtenTotal=0L;foreach($entry in $zip.Entries){if($entry.FullName -cnotin $payload -or $seen.ContainsKey($entry.FullName) -or $entry.Length -gt 134217728 -or (($entry.ExternalAttributes -shr 16) -band 0xF000) -eq 0xA000){throw '安装包含未知路径、重复项或链接。'};$seen[$entry.FullName]=$true;$expanded+=$entry.Length;if($expanded -gt 134217728){throw '解包大小超过允许范围。'};$sourceStream=$entry.Open();$output=[IO.File]::Open((Join-Path $Destination $entry.FullName),'CreateNew','Write','None');try{$buffer=New-Object byte[] 65536;$written=0L;while(($count=$sourceStream.Read($buffer,0,$buffer.Length)) -gt 0){$written+=$count;$writtenTotal+=$count;if($writtenTotal -gt 134217728 -or $written -gt $entry.Length){throw '解包内容超过声明大小。'};$output.Write($buffer,0,$count)};if($written -ne $entry.Length){throw '安装包条目不完整。'};$output.Flush($true)}finally{$output.Dispose();$sourceStream.Dispose()}}}finally{$zip.Dispose()}
    $sums=@();foreach($line in Get-Content -LiteralPath (Join-Path $Destination 'SHA256SUMS.txt')){if($line -cnotmatch '^([a-f0-9]{64})  ([A-Za-z0-9_.-]+)$' -or $Matches[2] -cnotin $payload -or $Matches[2] -ceq 'SHA256SUMS.txt'){throw '包内校验清单无效。'};$sums+=@{name=$Matches[2];sha256=$Matches[1]}}
    $sums+=@{name='SHA256SUMS.txt';sha256=Hash (Join-Path $Destination 'SHA256SUMS.txt')};Verify $Destination $sums
    $receipt=Json (Join-Path $Destination 'package-receipt.json');if($receipt.schema -ne 1 -or $receipt.product -cne 'CodexProxyTray' -or $receipt.version -cne $ExpectedVersion -or $receipt.source_commit -cnotmatch '^[a-f0-9]{40}$' -or $receipt.files.Count -ne 6){throw '程序来源记录无效。'}
    foreach($file in $receipt.files){if($file.name -cnotin $payload -or (Hash (Join-Path $Destination $file.name)) -cne $file.sha256){throw '核心程序校验失败。'}}
    if((Get-Item -LiteralPath (Join-Path $Destination 'CodexProxyTray.exe')).VersionInfo.ProductVersion -cne $ExpectedVersion){throw '程序版本与发布版本不一致。'}
    if($ExpectedSource -and $receipt.source_commit -cne $ExpectedSource){throw '程序来源与安装器编译来源不一致。'}
    $coreNames=@('CodexProxyTray.exe','codex-proxy-cli.exe','WebView2Loader.dll','LICENSE','THIRD_PARTY_NOTICES.md','flag-icons-license.txt');if(@(Compare-Object ($coreNames|Sort-Object) (@($receipt.files.name)|Sort-Object)).Count -ne 0){throw '核心程序清单不完整或含未知文件。'}
    [pscustomobject]@{files=@($sums);receipt=$receipt;source_commit=$receipt.source_commit}
}
$payload=@('CodexProxyTray.exe','codex-proxy-cli.exe','WebView2Loader.dll','LICENSE','THIRD_PARTY_NOTICES.md','flag-icons-license.txt','package-receipt.json','QUICKSTART.txt','fluent-icons-license.txt','SHA256SUMS.txt')
if($Phase -eq 'Library'){return}
if($Phase -notin @('Prepare','Apply') -or -not $Job){throw '更新阶段或任务目录缺失。'}
$Job=[IO.Path]::GetFullPath($Job);Plain $Job
$plan=Json (Join-Path $Job 'plan.json');$data=[IO.Path]::GetFullPath($plan.data_root);$exe=[IO.Path]::GetFullPath($plan.executable);$target=[IO.Path]::GetDirectoryName($exe);$installRoot=[IO.Path]::GetDirectoryName($target)
if($plan.schema -ne 1 -or $plan.product -cne 'CodexProxyTray' -or $plan.version -cnotmatch '^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$' -or [IO.Path]::GetFileName($exe) -cne 'CodexProxyTray.exe' -or [IO.Path]::GetDirectoryName($Job) -ine (Join-Path $data 'updates') -or $data -ine (Join-Path $env:LOCALAPPDATA 'CodexProxyTray') -or $target.StartsWith($data+'\',[StringComparison]::OrdinalIgnoreCase) -or $data.StartsWith($target+'\',[StringComparison]::OrdinalIgnoreCase)){throw '安装计划与本机助手不匹配。'}
Plain $target;Plain $installRoot;Plain $data
$id=[IO.Path]::GetFileName($Job);if($id -cnotmatch '^\d+-\d+$'){throw '更新任务标识无效。'}
$managed=([IO.Path]::GetFileName($target) -ieq 'current') -and (Test-Path -LiteralPath (Join-Path $installRoot 'active-version.json'))
$candidate=Join-Path $installRoot ('.candidate-update-'+$id);$obsolete=Join-Path $installRoot ('.obsolete-update-'+$id)
$previous=if($managed){Join-Path $installRoot 'previous'}else{Join-Path $installRoot ([IO.Path]::GetFileName($target)+'.previous')}
$activePath=if($managed){Join-Path $installRoot 'active-version.json'}else{Join-Path $installRoot ('.'+[IO.Path]::GetFileName($target)+'-active-version.json')}
$journalPath=if($managed){Join-Path $installRoot 'install-journal.json'}else{Join-Path $installRoot ('.'+[IO.Path]::GetFileName($target)+'-install-journal.json')}
$lockPath=Join-Path $installRoot ('.'+[IO.Path]::GetFileName($target)+'-update.lock')
$lock=$null;$committed=$false;$switched=$false;$movedPrevious=$false;$movedCurrent=$false;$new=$null
try {
    $lock=[IO.File]::Open($lockPath,'OpenOrCreate','ReadWrite','None')
    $parent=Get-Process -Id ([int]$plan.parent_pid) -ErrorAction SilentlyContinue
    if(-not(SameParent $parent)){throw '原助手身份已变化，未开始安装。'}
    $others=@(OwnProcesses $exe|Where-Object {$_.ProcessId -ne $parent.Id});if($others.Count){throw '另一个进程正在使用助手程序，未开始替换。'}
    if($Phase -eq 'Prepare'){
        if((Hash (Join-Path $Job 'package.zip')) -cne $plan.zip_sha256 -or (Hash (Join-Path $Job 'manifest.json')) -cne $plan.manifest_sha256){throw '官方附件 SHA-256 校验失败。'}
        $manifest=Json (Join-Path $Job 'manifest.json');if($manifest.schema -ne 1 -or $manifest.product -cne 'CodexProxyTray' -or $manifest.version -cne $plan.version){throw '发布清单与目标版本不匹配。'}
        $before=@(Inventory $target);if(-not($before.name -ccontains 'CodexProxyTray.exe') -or -not($before.name -ccontains 'codex-proxy-cli.exe')){throw '不是受支持的完整助手目录。'}
        $oldActive=$null;$oldPrevious=@();if(Test-Path -LiteralPath $activePath){$oldActive=Json $activePath;if($oldActive.schema -ne 1 -or $oldActive.current.path -ine $target){throw '活动版本记录与目录不一致。'};Verify $target $oldActive.current.files;CopyDurable $activePath (Join-Path $Job 'pre-active-version.json')}
        if(Test-Path -LiteralPath $journalPath){$journal=Json $journalPath;if($journal.state -notin @('installed','installed-partial','active','complete')){throw '已有未结束的安装恢复流程，请先处理。'};CopyDurable $journalPath (Join-Path $Job 'pre-install-journal.json')}
        if(Test-Path -LiteralPath $previous){if(-not $oldActive -or $oldActive.previous.path -ine $previous){throw '回退目录归属不明，已保留。'};foreach($reference in @($oldActive.recovery_references)){if(([string]$reference).StartsWith($previous,[StringComparison]::OrdinalIgnoreCase)){throw '已有恢复材料仍引用旧回退目录，已保留。'}};Verify $previous $oldActive.previous.files;NoCodeUse @($previous);$oldPrevious=@($oldActive.previous.files)}
        if(@(Get-ChildItem -LiteralPath $installRoot -Directory -Force|Where-Object {$_.Name -like '.candidate-update-*' -or $_.Name -like '.obsolete-update-*'}).Count){throw '已有更新暂存恢复目录，请先核实上次结果。'}
        if((Test-Path -LiteralPath $candidate) -or (Test-Path -LiteralPath $obsolete)){throw '更新暂存目录已存在，未覆盖。'}
        [void](New-Item -ItemType Directory -Path $candidate)
        $package=ExpandPayload (Join-Path $Job 'package.zip') $candidate $plan.version
        $sums=@($package.files);$receipt=$package.receipt
        Durable (Join-Path $Job 'verified.json') @{schema=1;before=$before;after=$sums;old_previous=$oldPrevious;source_commit=$receipt.source_commit;old_active=$oldActive;target=$target;previous=$previous;candidate=$candidate;obsolete=$obsolete}
        exit 0
    }
    $verified=Json (Join-Path $Job 'verified.json');if($verified.target -ine $target -or $verified.previous -ine $previous -or $verified.candidate -ine $candidate -or $verified.obsolete -ine $obsolete){throw '已校验计划路径发生变化。'}
    Verify $target $verified.before;Verify $candidate $verified.after;if(Test-Path -LiteralPath $previous){Verify $previous $verified.old_previous}
    Durable (Join-Path $Job 'handoff-ready.json') @{schema=1;pid=$PID;state='waiting-parent'}
    if(-not(WaitGone $parent.Id $plan.parent_created 30)){throw '助手尚未退出，已取消替换。'}
    if(@(OwnProcesses $exe).Count){throw '新的助手实例仍在运行，已取消替换。'}
    foreach($name in @('config.json','logs\events-v1.json')){$path=Join-Path $data $name;if(Test-Path -LiteralPath $path){Plain $path;CopyDurable $path (Join-Path $Job ([IO.Path]::GetFileName($name)));if((Hash $path) -cne (Hash (Join-Path $Job ([IO.Path]::GetFileName($name))))){throw '配置或日志备份未通过校验。'}}}
    $journal=@{schema=1;state='switching';current=$target;previous=$previous;source_commit=$verified.source_commit;recovery=$Job;candidate=$candidate;obsolete=$obsolete};Durable $journalPath $journal
    if(Test-Path -LiteralPath $previous){SafeMove $previous $obsolete;$movedPrevious=$true}
    SafeMove $target $previous;$movedCurrent=$true;SafeMove $candidate $target;$switched=$true
    $journal.state='awaiting-readiness';Durable $journalPath $journal
    $new=StartHelper
    if(-not(Ready $new)){throw '新版本未通过界面就绪检查。'}
    Verify $target $verified.after
    $references=@($verified.old_active.recovery_references)+@((Join-Path $Job 'config.json'),(Join-Path $Job 'events-v1.json'))|Where-Object {Test-Path -LiteralPath $_}
    $active=@{schema=1;state='active';current=@{path=$target;version=$plan.version;source_commit=$verified.source_commit;files=$verified.after};previous=@{path=$previous;files=$verified.before;source_commit=$verified.old_active.current.source_commit};readiness=@{native='passed';main_rendered=$true;tray_rendered=$true;helper_pid=$new.Id};recovery_references=@($references);historical_receipts=@((Join-Path $Job 'pre-active-version.json'),(Join-Path $Job 'pre-install-journal.json'));committed_at=[DateTime]::UtcNow.ToString('o');optional_service='unchanged'}
    Durable $activePath $active;$committed=$true
    Durable (Join-Path $Job 'receipt-lifecycle.json') @{schema=1;state='superseded-after-durable-readiness';active_receipt=$activePath;active_receipt_sha256=Hash $activePath;historical_receipts=$active.historical_receipts;retained_rollback=$previous;retired_previous=@{path=$obsolete;files=$verified.old_previous;recovery_status='not-a-rollback-target'};protected_recovery_references=$verified.old_active.recovery_references}
    $journal.state='installed';$journal.obsolete=$null;$journal.candidate=$null;Durable $journalPath $journal
    if($movedPrevious){RemoveCode $obsolete $verified.old_previous}
    foreach($name in @('package.zip','release-sums.txt','manifest.json')){$download=Join-Path $Job $name;if(Test-Path -LiteralPath $download){Plain $download;Remove-Item -LiteralPath $download}}
    if(Test-Path -LiteralPath (Join-Path $installRoot 'installation.json')){
        $registration=StartHelper '--register-installation';try{if(-not $registration.WaitForExit(20000) -or $registration.ExitCode -ne 0){throw '新版本已健康运行，但 Windows 安装登记同步未完成；请重新运行安装器补登记。'}}finally{$registration.Dispose()}
    }
    Result 'installed' $null
} catch {
    $failure=$_.Exception.Message
    if($committed){$journal.state='cleanup-blocked';$journal.obsolete=if(Test-Path -LiteralPath $obsolete){$obsolete}else{$null};Durable $journalPath $journal;Result 'installed_cleanup_pending' ($failure+' 新版本运行正常，安装收尾未完成；恢复材料已保留。')}elseif($switched){
        $canRollback=$true
        if($new -and -not $new.HasExited){$created=$new.StartTime.ToUniversalTime().ToFileTimeUtc().ToString();$quitter=StartHelper '--quit';[void]$quitter.WaitForExit(10000);$canRollback=WaitGone $new.Id $created 20}
        if($canRollback -and -not @(OwnProcesses $exe).Count){
            try{SafeMove $target $candidate;SafeMove $previous $target;if($movedPrevious){SafeMove $obsolete $previous};$configBackup=Join-Path $Job 'config.json';if(Test-Path -LiteralPath $configBackup){if(Test-Path -LiteralPath (Join-Path $data 'config.json')){CopyDurable (Join-Path $data 'config.json') (Join-Path $Job 'config-after-failed-update.json')};RestoreConfig $configBackup (Join-Path $data 'config.json')};foreach($pair in @(@('pre-active-version.json',$activePath),@('pre-install-journal.json',$journalPath))){if(Test-Path -LiteralPath (Join-Path $Job $pair[0])){Durable $pair[1] (Json (Join-Path $Job $pair[0]))}};$old=StartHelper;if(-not(Ready $old)){throw '回退程序未通过就绪检查。'};Result 'rolled_back' ($failure+' 已恢复上一版本；配置、日志及失败材料保留在本机。')}catch{Result 'error' ($failure+' 自动回退未完成，请保留恢复目录并手动处理。')}
        }else{Result 'error' ($failure+' 新助手尚未正常退出，未强制关闭或继续替换。')}
    }elseif($movedCurrent){try{SafeMove $previous $target;if($movedPrevious){SafeMove $obsolete $previous};$old=StartHelper;if(-not(Ready $old)){throw '回退程序未通过就绪检查。'};Result 'rolled_back' $failure}catch{Result 'error' ($failure+' 代码切换恢复未完成。')}}
    else{Result 'error' $failure}
    exit 1
} finally {if($lock){$lock.Dispose()}}
