param([Parameter(Mandatory=$true)][string]$EvidenceDirectory)
$ErrorActionPreference='Stop'
# This fixture deliberately uses the real fixed per-user paths. Never run it on a developer PC.
if(-not $IsWindows -or $env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or [IntPtr]::Size -ne 8){throw 'Installer bootstrap acceptance requires a fresh GitHub-hosted x64 Windows runner.'}
if(-not $env:RUNNER_TEMP -or $env:GITHUB_SHA -cnotmatch '^[0-9a-f]{40}$'){throw 'Missing isolated runner identity.'}
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$temp=[IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\')
$evidence=[IO.Path]::GetFullPath($EvidenceDirectory)
if(-not $evidence.StartsWith($temp+'\',[StringComparison]::OrdinalIgnoreCase) -or (Test-Path -LiteralPath $evidence)){throw 'Evidence must be a new directory beneath RUNNER_TEMP.'}
function PlainParents([string]$Path){
    $cursor=[IO.Path]::GetFullPath($Path)
    while($cursor){if(Test-Path -LiteralPath $cursor){if((Get-Item -LiteralPath $cursor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint){throw 'Reparse-point fixture paths are refused.'}};$next=[IO.Path]::GetDirectoryName($cursor);if($next -eq $cursor){break};$cursor=$next}
}
PlainParents $evidence
$local=[Environment]::GetFolderPath('LocalApplicationData')
if(-not $local -or [IO.Path]::GetFullPath($env:LOCALAPPDATA).TrimEnd('\') -ine $local.TrimEnd('\')){throw 'Known-folder and process LocalAppData must match.'}
$installRoot=Join-Path $local 'Programs\CodexProxyTray';$data=Join-Path $local 'CodexProxyTray'
$registry=@('HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexProxyTray','HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexProxyTray')
$menu=[Environment]::GetFolderPath('Programs')
$shortcuts=@('CodexProxyTray.lnk','CodexProxyTray 更新通知.lnk','codex-proxy-cli.lnk')|ForEach-Object {Join-Path $menu $_}
$work=Join-Path $temp ('CodexProxyTray-bootstrap-'+[Guid]::NewGuid().ToString('N'))
$checks=[Collections.Generic.List[object]]::new()
$report=[ordered]@{schema=1;status='running';source_commit=$env:GITHUB_SHA;scope='Real NSIS to native x64 PowerShell to unknown-directory refusal; synthetic empty ZIP only. No GUI, real package, registration, services or user data.';checks=$checks;cleanup='not_started';outcomes=@()}
function Require([bool]$Condition,[string]$Name){if(-not $Condition){throw $Name};$checks.Add(@{name=$Name;status='passed'})}
function Hash([string]$Path){(Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()}
function Redact([string]$Text){foreach($prefix in @($work,$installRoot,$data,$local,$env:USERPROFILE)){if($prefix){$Text=$Text.Replace($prefix,'<owned-path>')}};$Text=[regex]::Replace($Text,'S-\d-(?:\d+-)*\d+','<SID>');if($Text.Length -gt 4096){$Text=$Text.Substring(0,4096)};$Text}
function RunBootstrap([string]$File){
    $info=[Diagnostics.ProcessStartInfo]::new($File);$info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.WindowStyle='Hidden';$info.WorkingDirectory=$work
    $info.ArgumentList.Add('/S');$info.RedirectStandardOutput=$true;$info.RedirectStandardError=$true;$info.StandardOutputEncoding=[Text.Encoding]::Unicode
    $process=[Diagnostics.Process]::Start($info)
    # Drain both redirected streams concurrently with fixed buffers and capped retained text.
    $streams=@($process.StandardOutput,$process.StandardError)|ForEach-Object {@{reader=$_;buffer=[char[]]::new(1024);pending=$null;text=[Text.StringBuilder]::new();done=$false;overflow=$false}}
    foreach($stream in $streams){$stream.pending=$stream.reader.ReadAsync($stream.buffer,0,$stream.buffer.Length)}
    $deadline=[DateTime]::UtcNow.AddSeconds(120);$drainDeadline=$null;$timedOut=$false
    try{
        while($true){
            foreach($stream in $streams){
                if(-not $stream.done -and $stream.pending.IsCompleted){
                    $count=$stream.pending.GetAwaiter().GetResult()
                    if($count -eq 0){$stream.done=$true;continue}
                    $retain=[Math]::Min($count,8192-$stream.text.Length)
                    if($retain -gt 0){[void]$stream.text.Append($stream.buffer,0,$retain)}
                    if($retain -lt $count){$stream.overflow=$true}
                    $stream.pending=$stream.reader.ReadAsync($stream.buffer,0,$stream.buffer.Length)
                }
            }
            if($process.HasExited){if(-not $drainDeadline){$drainDeadline=[DateTime]::UtcNow.AddSeconds(2)};if(@($streams|Where-Object {-not $_.done}).Count -eq 0 -or [DateTime]::UtcNow -ge $drainDeadline){break}}
            if([DateTime]::UtcNow -ge $deadline){$timedOut=$true;break}
            Start-Sleep -Milliseconds 20
        }
        $diagnostic=($streams|ForEach-Object {$_.text.ToString()+$(if(-not $_.done){'<diagnostic stream not completed>'})}) -join "`n"
        $script:report.outcomes+=@{exit_code=$(if($process.HasExited){$process.ExitCode}else{$null});timed_out=$timedOut;bootstrap_output=(Redact $diagnostic);streams_completed=(@($streams|Where-Object {-not $_.done}).Count -eq 0);output_within_limit=(@($streams|Where-Object overflow).Count -eq 0)}
        if($timedOut){throw 'bootstrap_timeout_scene_retained'}
        Require ($report.outcomes[-1].streams_completed -and $report.outcomes[-1].output_within_limit) 'bootstrap_diagnostics_bounded_and_complete'
        return $process.ExitCode
    }finally{$process.Dispose()}
}
[void][IO.Directory]::CreateDirectory($evidence)
try{
    foreach($path in @($installRoot,$data)+$registry+$shortcuts){Require (-not (Test-Path -LiteralPath $path)) 'fresh_fixed_user_paths_and_registration'}
    Require (@(Get-Process -Name CodexProxyTray -ErrorAction SilentlyContinue).Count -eq 0) 'no_preexisting_gui_touched'
    PlainParents $installRoot;PlainParents $data;PlainParents $work
    $package=Get-Content -LiteralPath (Join-Path $repo 'package.json') -Raw -Encoding utf8|ConvertFrom-Json
    $version=[string]$package.version;Require ($version -cmatch '^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$') 'current_source_version'
    $report.version=$version
    $compiler=Get-Command makensis.exe -ErrorAction SilentlyContinue|Select-Object -First 1
    $compilerPath=if($compiler){$compiler.Source}else{Join-Path ${env:ProgramFiles(x86)} 'NSIS\makensis.exe'}
    Require (Test-Path -LiteralPath $compilerPath -PathType Leaf) 'nsis_compiler_available'
    [void][IO.Directory]::CreateDirectory($work)
    $zip=Join-Path $work "CodexProxyTray-v$version-windows-x64.zip"
    $file=[IO.File]::Open($zip,'CreateNew','ReadWrite','None')
    try{$archive=[IO.Compression.ZipArchive]::new($file,[IO.Compression.ZipArchiveMode]::Create,$true);$archive.Dispose()}finally{$file.Dispose()}
    $archive=[IO.Compression.ZipFile]::OpenRead($zip);try{Require ($archive.Entries.Count -eq 0 -and (Get-Item -LiteralPath $zip).Length -eq 22) 'synthetic_valid_empty_zip_only'}finally{$archive.Dispose()}
    foreach($name in @('install_application.ps1','install_update.ps1')){[IO.File]::WriteAllText((Join-Path $work $name),[IO.File]::ReadAllText((Join-Path $repo ('src-tauri/src/'+$name)),[Text.Encoding]::UTF8),[Text.UTF8Encoding]::new($true))}
    $setup=Join-Path $work 'bootstrap-probe.exe'
    $arguments=@('/V2','/WX',"/DRELEASE_DIR=$work","/DOUTPUT_FILE=$setup","/DAPP_VERSION=$version","/DSOURCE_COMMIT=$env:GITHUB_SHA","/DZIP_SHA256=$(Hash $zip)","/DENGINE_PATH=$(Join-Path $work 'install_application.ps1')","/DUPDATE_ENGINE_PATH=$(Join-Path $work 'install_update.ps1')","/DICON_PATH=$(Join-Path $repo 'src-tauri/icons/icon.ico')",(Join-Path $repo 'scripts/windows-installer.nsi'))
    & $compilerPath @arguments
    Require ($LASTEXITCODE -eq 0 -and (Test-Path -LiteralPath $setup -PathType Leaf)) 'current_nsis_with_all_eight_macros_compiled'
    $file=[IO.File]::OpenRead($setup);$reader=[IO.BinaryReader]::new($file)
    try{$file.Position=0x3c;$offset=$reader.ReadUInt32();$file.Position=$offset;Require ($reader.ReadUInt32() -eq 0x4550 -and $reader.ReadUInt16() -eq 0x14c) 'probe_wrapper_is_x86_for_sysnative_alias'}finally{$reader.Dispose()}
    foreach($path in @($installRoot,$data)+$registry+$shortcuts){Require (-not (Test-Path -LiteralPath $path)) 'fixed_paths_still_fresh_before_fixture_creation'}
    Require (@(Get-Process -Name CodexProxyTray -ErrorAction SilentlyContinue).Count -eq 0) 'still_no_preexisting_gui_touched'
    [void][IO.Directory]::CreateDirectory($data)
    $config=Join-Path $data 'config.json'
    [IO.File]::WriteAllText($config,'{"launch_target":"none","network_enabled":false,"auto_check_updates":false,"connectivity_enabled":false,"traffic_enabled":false}',[Text.UTF8Encoding]::new($false));$configHash=Hash $config
    [void][IO.Directory]::CreateDirectory($installRoot)
    $note=Join-Path $installRoot 'bootstrap-owned-unknown.txt';$noteText='Synthetic bootstrap refusal fixture; not an installed product.'
    [IO.File]::WriteAllText($note,$noteText,[Text.UTF8Encoding]::new($false));$noteHash=Hash $note
    $started=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    $exitCode=RunBootstrap $setup
    $resultPath=Join-Path $data 'installation-result-v1.json'
    Require ((Test-Path -LiteralPath $resultPath -PathType Leaf) -and (Get-Item -LiteralPath $resultPath).Length -le 16384) 'actual_backend_result_present_and_bounded'
    $result=[IO.File]::ReadAllText($resultPath,[Text.Encoding]::UTF8)|ConvertFrom-Json
    $report.outcomes[-1].result=@{schema=$result.schema;action=$result.action;status=$result.status;phase=$result.phase;version=$result.version;error=(Redact ([string]$result.error))}
    Require ($exitCode -eq 2 -and $result.schema -eq 1 -and $result.product -ceq 'CodexProxyTray.Installation' -and $result.action -ceq 'Install' -and $result.status -ceq 'failed' -and $result.phase -ceq 'error' -and $result.version -ceq $version -and $result.root -ieq $installRoot -and $result.completed_at -ge $started -and $result.error -cmatch '目标目录非空且没有本产品归属记录') 'real_native_x64_backend_refused_unknown_directory'
    $entries=@(Get-ChildItem -LiteralPath $installRoot -Force)
    Require ($entries.Count -eq 1 -and $entries[0].Name -ceq 'bootstrap-owned-unknown.txt' -and -not $entries[0].PSIsContainer -and -not ($entries[0].Attributes -band [IO.FileAttributes]::ReparsePoint) -and (Hash $note) -ceq $noteHash -and (Hash $config) -ceq $configHash) 'note_and_synthetic_config_unchanged_no_payload_extracted'
    foreach($path in $registry+$shortcuts){Require (-not (Test-Path -LiteralPath $path)) 'no_registration_or_shortcut_created'}
    Require (@(Get-Process -Name CodexProxyTray -ErrorAction SilentlyContinue).Count -eq 0) 'no_gui_started'
    # Delete only the single proved-owned note and its now-empty fixed root.
    PlainParents $note;[IO.File]::Delete($note);[IO.Directory]::Delete($installRoot)
    $allowed=@([IO.Path]::GetFileName($zip),'install_application.ps1','install_update.ps1','bootstrap-probe.exe')
    PlainParents $work;$entries=@(Get-ChildItem -LiteralPath $work -Force)
    Require ($entries.Count -eq $allowed.Count -and @($entries|Where-Object {$_.PSIsContainer -or $_.Name -cnotin $allowed -or ($_.Attributes -band [IO.FileAttributes]::ReparsePoint)}).Count -eq 0) 'owned_temporary_file_allowlist'
    foreach($entry in $entries){[IO.File]::Delete($entry.FullName)};[IO.Directory]::Delete($work)
    $report.cleanup='owned_note_empty_root_and_exact_temp_files_removed_synthetic_data_result_retained';$report.status='passed'
}catch{$report.status='failed';$report.error=Redact $_.Exception.Message;$report.cleanup='retained_for_diagnostics';throw}
finally{$report.finished_at=[DateTime]::UtcNow.ToString('o');[IO.File]::WriteAllText((Join-Path $evidence 'bootstrap-checks.json'),($report|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))}
