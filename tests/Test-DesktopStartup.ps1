param([Parameter(Mandatory=$true)][string]$BinaryPath,[Parameter(Mandatory=$true)][string]$EvidenceDirectory)
$ErrorActionPreference='Stop'
if($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted'){throw 'Run this isolated desktop startup check only on a GitHub-hosted runner.'}
$BinaryPath=(Resolve-Path -LiteralPath $BinaryPath).Path
if([IO.Path]::GetFileName($BinaryPath) -ne 'CodexProxyTray.exe'){throw 'Select the desktop executable.'}
if(Test-Path -LiteralPath $EvidenceDirectory){throw 'Existing evidence is preserved.'}
New-Item -ItemType Directory -Path $EvidenceDirectory|Out-Null
$sandbox=Join-Path $env:RUNNER_TEMP ('CodexProxyTray-desktop-'+[Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $sandbox|Out-Null
$root=Join-Path $sandbox 'CodexProxyTray';New-Item -ItemType Directory -Path $root|Out-Null
[IO.File]::WriteAllText((Join-Path $root 'config.json'),'{"launch_target":"none","network_enabled":false,"connectivity_enabled":false,"traffic_enabled":false,"auto_check_updates":false}')
$report=[ordered]@{format='codex-proxy-tray-desktop-startup-v1';status='running';started_at=[DateTime]::UtcNow.ToString('o');binary_sha256=(Get-FileHash -LiteralPath $BinaryPath).Hash.ToLowerInvariant();scope='Isolated real WebView2 startup and frontend IPC; no UI automation, screenshots, network probes, service installation or real accounts';views=@();cleanup='pending'}
function Start-Owned([string]$Arguments){$info=[Diagnostics.ProcessStartInfo]::new();$info.FileName=$BinaryPath;$info.Arguments=$Arguments;$info.WorkingDirectory=[IO.Path]::GetDirectoryName($BinaryPath);$info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.WindowStyle='Hidden';$info.EnvironmentVariables['LOCALAPPDATA']=$sandbox;$info.EnvironmentVariables['WEBVIEW2_USER_DATA_FOLDER']=Join-Path $sandbox 'webview-data';$info.RedirectStandardError=$true;return [Diagnostics.Process]::Start($info)}
$process=$null;$failure=$null
try{
    if(@(Get-Process -Name CodexProxyTray -ErrorAction SilentlyContinue).Count){throw 'Another desktop helper exists; it was not touched.'}
    $process=Start-Owned '--background';$stderr=$process.StandardError.ReadToEndAsync()
    $deadline=[DateTime]::UtcNow.AddSeconds(45)
    while($true){
        if($process.HasExited){$detail=$stderr.GetAwaiter().GetResult();[IO.File]::WriteAllText((Join-Path $EvidenceDirectory 'startup.stderr.log'),$detail);throw ('Desktop exited before readiness: '+$process.ExitCode)}
        if(Test-Path -LiteralPath (Join-Path $root 'ui-error.json')){throw 'The real frontend reported a render error.'}
        $ready=@();foreach($view in @('main','tray')){$path=Join-Path $root ('ui-ready-'+$view+'.json');if(Test-Path -LiteralPath $path){try{$value=Get-Content -LiteralPath $path -Raw|ConvertFrom-Json;if($value.rendered -and $value.helper_pid -eq $process.Id -and $value.window -eq $view){$ready+=$view}}catch{}}}
        if($ready.Count -eq 2){$report.views=$ready;break}
        if([DateTime]::UtcNow -ge $deadline){throw 'Real main/tray frontend readiness timed out.'}
        Start-Sleep -Milliseconds 200
    }
    Start-Sleep -Seconds 3
    if($process.HasExited -or (Test-Path -LiteralPath (Join-Path $root 'ui-error.json'))){throw 'The frontend failed after receiving backend state.'}
    foreach($view in @('main','tray')){Copy-Item -LiteralPath (Join-Path $root ('ui-ready-'+$view+'.json')) -Destination (Join-Path $EvidenceDirectory ('layout-'+$view+'.json'))}
    $layout=Get-Content -LiteralPath (Join-Path $root 'ui-ready-main.json') -Raw|ConvertFrom-Json
    if(@($layout.geometry.body_children|Where-Object {$_.class -match '(^|\s)app-shell(\s|$)'}).Count){throw 'Fluent portal copied the page layout and can cover the real interface.'}
    if(-not $layout.geometry.layers[0].nodes.Count -or $layout.geometry.layers[0].nodes[0].class -match '(^|\s)app-shell(\s|$)'){throw 'A full-page layer covers the sidebar navigation.'}
    foreach($selector in @('.sidebar','h1','.main-scroll')){$node=@($layout.geometry.nodes|Where-Object selector -eq $selector);if($node.Count -ne 1 -or $node[0].width -le 0 -or $node[0].height -le 0 -or $node[0].visibility -ne 'visible'){throw "Core page element is not visible: $selector"}}
    $grid=@($layout.geometry.nodes|Where-Object selector -eq '.dashboard-grid')[0];$guard=@($layout.geometry.nodes|Where-Object selector -eq '.guard-floating')[0]
    if($null -eq $guard -or [Math]::Abs(($guard.x+$guard.width)-($grid.x+$grid.width)) -gt 1.5){throw 'The guard control must align with the content right edge.'}
    $report['guard_content_alignment_verified']=$true
    $plot=@($layout.geometry.nodes|Where-Object selector -eq '.traffic-plot')[0];$chart=@($layout.geometry.nodes|Where-Object selector -eq '.traffic-chart-card .probe-chart')[0]
    if($plot.width -gt 0 -and [Math]::Abs(($plot.x-$chart.x)-(($chart.x+$chart.width)-($plot.x+$plot.width))) -gt 1.5){throw 'The traffic plot must have equal left and right gutters.'}
    $report['traffic_plot_symmetric_gutters_verified']=($plot.width -gt 0)

    $report['page_occlusion_check']='passed'
    $status=Get-Content -LiteralPath (Join-Path $root 'status.json') -Raw|ConvertFrom-Json;if($status.enabled){throw 'A first launch must remain paused until explicitly enabled.'};$report['guard_default_paused']=$true
    $quitter=Start-Owned '--quit';try{if(-not $quitter.WaitForExit(10000) -or $quitter.ExitCode -ne 0){throw 'Quit request failed.'}}finally{$quitter.Dispose()}
    if(-not $process.WaitForExit(15000) -or $process.ExitCode -ne 0){throw 'Desktop did not exit cleanly.'}
    $guardPath=Join-Path $root 'guard-v1.json'
    if(Test-Path -LiteralPath $guardPath){throw 'Shutdown must not save a manual guard preference.'}
    foreach($enabled in @($true,$false)){
        $process.Dispose();$process=$null;[IO.File]::WriteAllText($guardPath,(@{schema=1;enabled=$enabled}|ConvertTo-Json -Compress))
        $process=Start-Owned '--background';$stderr=$process.StandardError.ReadToEndAsync();$deadline=[DateTime]::UtcNow.AddSeconds(45)
        do{if($process.HasExited){throw 'Guard preference restart exited early.'};try{$status=Get-Content -LiteralPath (Join-Path $root 'status.json') -Raw|ConvertFrom-Json}catch{$status=$null};if($status.helper_pid -eq $process.Id -and $status.backend_ready){break};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $deadline)
        if($status.helper_pid -ne $process.Id -or -not $status.backend_ready -or $status.enabled -ne $enabled){throw 'A restart did not restore the saved guard preference.'}
        $quitter=Start-Owned '--quit';try{if(-not $quitter.WaitForExit(10000) -or $quitter.ExitCode -ne 0){throw 'Guard preference quit failed.'}}finally{$quitter.Dispose()}
        if(-not $process.WaitForExit(15000) -or $process.ExitCode -ne 0){throw 'Guard preference process did not exit gracefully.'}
        $saved=Get-Content -LiteralPath $guardPath -Raw|ConvertFrom-Json;if($saved.enabled -ne $enabled){throw 'Runtime shutdown overwrote the manual guard preference.'}
    }
    $report['guard_restart_preferences_and_shutdown_verified']=$true
    $report.status='passed'
}catch{$failure=$_;$report.status='failed';$report.error=$_.Exception.Message}
finally{
    if($process){if(-not $process.HasExited){$process.Kill($true);[void]$process.WaitForExit(10000);$report.status='failed';if(-not $failure){$failure='Owned process required forced cleanup.'}};$process.Dispose()}
    if($report.status -eq 'passed'){
        $absolute=[IO.Path]::GetFullPath($sandbox);$temporaryRoot=[IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\')+'\'
        if(-not $absolute.StartsWith($temporaryRoot,[StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetFileName($absolute) -notlike 'CodexProxyTray-desktop-*' -or ((Get-Item -LiteralPath $absolute).Attributes -band [IO.FileAttributes]::ReparsePoint)){throw 'Sandbox cleanup boundary mismatch.'}
        try{Start-Sleep -Seconds 2;Remove-Item -LiteralPath $absolute -Recurse -Force;$report.cleanup='passed'}catch{$report.cleanup='blocked';$report.status='failed';$report.error='Owned sandbox cleanup failed.';$failure=$_}
    }else{$report.cleanup='retained-for-debugging';if(Test-Path -LiteralPath (Join-Path $root 'ui-error.json')){Copy-Item -LiteralPath (Join-Path $root 'ui-error.json') -Destination (Join-Path $EvidenceDirectory 'ui-error.json')}}
    $report.finished_at=[DateTime]::UtcNow.ToString('o')
    [IO.File]::WriteAllText((Join-Path $EvidenceDirectory 'desktop-startup.json'),($report|ConvertTo-Json -Depth 6))
}
if($failure){throw $failure}
Write-Output 'Real main/tray WebView2 startup and graceful exit passed.'
