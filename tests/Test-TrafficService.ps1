#requires -Version 7.2
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$BinaryPath,
    [Parameter(Mandatory=$true)][string]$EvidenceDirectory,
    [ValidateRange(5,60)][int]$ProcessTimeoutSeconds=45,
    [ValidateRange(5,30)][int]$ServiceTimeoutSeconds=15
)
$ErrorActionPreference='Stop'
if(-not $IsWindows -or $env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_OS -ne 'Windows' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') {
    throw 'This check runs only on an isolated GitHub-hosted Windows runner; it must not run on a developer machine.'
}

$serviceName='CodexProxyTrayTraffic'
$displayName='CodexProxyTray Traffic Collector'
$imageName='CodexProxyTrayTrafficService.exe'
$loaderName='WebView2Loader.dll'
$loaderHash='86545b66cdb0603bc26b626fb9ad610cb6e71f28d468f5ea66df23b03dda96d5'
$sourceRoot=[IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
function Require([bool]$Condition,[string]$Code) {if(-not $Condition){throw $Code}}
function Full-Path([string]$Value) {
    $provider=$null;$drive=$null
    $path=$ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Value,[ref]$provider,[ref]$drive)
    Require ($provider.Name -eq 'FileSystem') 'path_provider_invalid'
    return [IO.Path]::GetFullPath($path)
}
function Is-Within([string]$Path,[string]$Directory) {return $Path.StartsWith($Directory.TrimEnd('\','/')+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)}
function Same-Path([string]$Left,[string]$Right) {return [string]::Equals((Full-Path $Left),(Full-Path $Right),[StringComparison]::OrdinalIgnoreCase)}
function No-Reparse([string]$Path) {Require (-not ((Get-Item -LiteralPath $Path -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) 'reparse_target_preserved'}
function Check-EvidencePath([string]$Path,[switch]$AllowExisting) {
    $absolute=Full-Path $Path
    Require (-not ($absolute -eq $sourceRoot -or (Is-Within $absolute $sourceRoot))) 'evidence_inside_checkout'
    $boundaries=@([IO.Path]::GetTempPath(),$env:RUNNER_TEMP)|Where-Object {$_}|ForEach-Object {Full-Path $_}
    $boundary=$boundaries|Where-Object {Is-Within $absolute $_}|Select-Object -First 1
    Require ([bool]$boundary) 'evidence_not_in_runner_temp'
    if(-not $AllowExisting){Require (-not (Test-Path -LiteralPath $absolute)) 'evidence_already_exists'}
    $cursor=[IO.DirectoryInfo]::new($absolute)
    while($cursor){if($cursor.Exists){No-Reparse $cursor.FullName};if($cursor.FullName.TrimEnd('\').Equals($boundary.TrimEnd('\'),[StringComparison]::OrdinalIgnoreCase)){break};$cursor=$cursor.Parent}
    return $absolute
}
function Write-NewJson([string]$Path,$Value) {
    $stream=[IO.File]::Open($Path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    $writer=[IO.StreamWriter]::new($stream,[Text.UTF8Encoding]::new($false))
    try {$writer.Write(($Value|ConvertTo-Json -Depth 8))} finally {$writer.Dispose()}
}
function Failure-Code($ErrorRecord) {
    $exception=$ErrorRecord.Exception
    while($exception){if($exception.Message -cmatch '^[a-z][a-z0-9_]{1,90}$'){return $exception.Message};$exception=$exception.InnerException}
    return 'unexpected_windows_error'
}
function Short-Diagnostic([string]$Path) {
    if(-not (Test-Path -LiteralPath $Path -PathType Leaf)){return ''}
    No-Reparse $Path
    $stream=[IO.File]::Open($Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite)
    try {$bytes=[byte[]]::new(8192);$count=$stream.Read($bytes,0,$bytes.Length);$text=[Text.Encoding]::UTF8.GetString($bytes,0,$count)} finally {$stream.Dispose()}
    foreach($value in @($binary,$sourceLoader,$serviceImage,$serviceDirectory,$productDirectory,$evidenceRoot,$diagnosticRoot,$env:USERPROFILE,$env:USERNAME,[Security.Principal.WindowsIdentity]::GetCurrent().Name)|Where-Object {$_}|Sort-Object Length -Descending){$text=[regex]::Replace($text,[regex]::Escape($value),'<REDACTED>',[Text.RegularExpressions.RegexOptions]::IgnoreCase)}
    $text=[regex]::Replace($text,'(?i)S-\d-(?:\d+-)*\d+','<SID>')
    $text=[regex]::Replace($text,'(?i)(?:[A-Z]:[\\/]|\\\\)[^\r\n<>]*','<PATH>')
    $text=[regex]::Replace($text,'(?im)^[A-Z][A-Z0-9_]{2,}\s*=.*$','<ENVIRONMENT_LINE>')
    $text=[regex]::Replace($text,'[\x00-\x08\x0b\x0c\x0e-\x1f]','')
    return $text.Trim().Substring(0,[Math]::Min(1200,$text.Trim().Length))
}
function Remove-Diagnostics {
    if(-not $diagnosticOwned){return}
    Require (Same-Path ((Get-Item -LiteralPath $diagnosticRoot -Force).FullName) $diagnosticRoot) 'diagnostic_directory_changed'
    No-Reparse $diagnosticRoot
    $entries=@(Get-ChildItem -LiteralPath $diagnosticRoot -Force)
    foreach($entry in $entries){Require (-not $entry.PSIsContainer -and $diagnosticFiles.Contains($entry.FullName)) 'unknown_diagnostic_file_preserved';No-Reparse $entry.FullName}
    foreach($entry in $entries){[IO.File]::Delete($entry.FullName)}
    [IO.Directory]::Delete($diagnosticRoot,$false)
}

if(-not ('CodexProxyTray.ServiceQaNative' -as [type])) {Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Security.Principal;
using System.Text;
namespace CodexProxyTray {
 public sealed class ServiceQaInfo {
  public uint Type,StartType,ErrorControl,State,ProcessId,Win32Exit,SpecificExit,Controls;
  public string Binary,Account,Display;
  public bool NoDependencies,NoLoadGroup;
  public byte[] Security;
 }
 public static class ServiceQaNative {
  [StructLayout(LayoutKind.Sequential)] struct Config {public uint type,start,error;public IntPtr binary,group;public uint tag;public IntPtr dependencies,account,display;}
  [StructLayout(LayoutKind.Sequential)] struct Status {public uint type,state,controls,exit,specific,checkpoint,hint,pid,flags;}
  [DllImport("shell32.dll",CharSet=CharSet.Unicode)] static extern int SHGetFolderPathW(IntPtr window,int folder,IntPtr token,uint flags,StringBuilder path);
  [DllImport("advapi32.dll",SetLastError=true)] static extern bool GetTokenInformation(IntPtr token,int kind,out uint information,uint length,out uint returned);
  [DllImport("advapi32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern IntPtr OpenSCManagerW(string machine,string database,uint access);
  [DllImport("advapi32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern IntPtr OpenServiceW(IntPtr manager,string name,uint access);
  [DllImport("advapi32.dll")] static extern bool CloseServiceHandle(IntPtr handle);
  [DllImport("advapi32.dll",SetLastError=true)] static extern bool QueryServiceConfigW(IntPtr service,IntPtr config,uint length,out uint needed);
  [DllImport("advapi32.dll",SetLastError=true)] static extern bool QueryServiceStatusEx(IntPtr service,int kind,out Status status,uint length,out uint needed);
  [DllImport("advapi32.dll",SetLastError=true)] static extern bool QueryServiceObjectSecurity(IntPtr service,uint info,byte[] descriptor,uint length,out uint needed);
  [DllImport("kernel32.dll",SetLastError=true)] static extern IntPtr OpenProcess(uint access,bool inherit,uint pid);
  [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
  [DllImport("advapi32.dll",SetLastError=true)] static extern bool GetKernelObjectSecurity(IntPtr handle,uint info,byte[] descriptor,uint length,out uint needed);
  public static bool Elevated() {using(var user=WindowsIdentity.GetCurrent()){uint elevated,returned;if(!GetTokenInformation(user.Token,20,out elevated,4,out returned))throw new Win32Exception();return elevated!=0;}}
  public static string ProgramFiles() {var path=new StringBuilder(260);if(SHGetFolderPathW(IntPtr.Zero,0x26,IntPtr.Zero,0,path)<0)throw new InvalidOperationException("known_folder_failed");return path.ToString();}
  public static byte[] ProcessSecurity(uint pid) {
   IntPtr process=OpenProcess(0x20000,false,pid);if(process==IntPtr.Zero)throw new Win32Exception();
   try {uint needed;GetKernelObjectSecurity(process,4,null,0,out needed);if(needed==0||needed>65536)throw new InvalidOperationException("process_security_size");var security=new byte[(int)needed];if(!GetKernelObjectSecurity(process,4,security,needed,out needed))throw new Win32Exception();return security;}
   finally {CloseHandle(process);}
  }
  public static ServiceQaInfo Inspect(string name) {
   IntPtr manager=OpenSCManagerW(null,null,1);if(manager==IntPtr.Zero)throw new Win32Exception();IntPtr service=IntPtr.Zero,memory=IntPtr.Zero;
   try {
    service=OpenServiceW(manager,name,0x20005);if(service==IntPtr.Zero){int code=Marshal.GetLastWin32Error();if(code==1060)return null;throw new Win32Exception(code);}
    uint needed;QueryServiceConfigW(service,IntPtr.Zero,0,out needed);if(needed<64||needed>65536)throw new InvalidOperationException("service_config_size");
    memory=Marshal.AllocHGlobal((int)needed);if(!QueryServiceConfigW(service,memory,needed,out needed))throw new Win32Exception();var config=(Config)Marshal.PtrToStructure(memory,typeof(Config));
    Status status;if(!QueryServiceStatusEx(service,0,out status,(uint)Marshal.SizeOf(typeof(Status)),out needed))throw new Win32Exception();
    QueryServiceObjectSecurity(service,5,null,0,out needed);if(needed==0||needed>65536)throw new InvalidOperationException("service_security_size");var security=new byte[needed];if(!QueryServiceObjectSecurity(service,5,security,needed,out needed))throw new Win32Exception();
    return new ServiceQaInfo {Type=config.type,StartType=config.start,ErrorControl=config.error,State=status.state,ProcessId=status.pid,Win32Exit=status.exit,SpecificExit=status.specific,Controls=status.controls,Binary=Marshal.PtrToStringUni(config.binary),Account=Marshal.PtrToStringUni(config.account),Display=Marshal.PtrToStringUni(config.display),NoDependencies=config.dependencies==IntPtr.Zero||Marshal.ReadInt16(config.dependencies)==0,NoLoadGroup=config.group==IntPtr.Zero||Marshal.ReadInt16(config.group)==0,Security=security};
   } finally {if(memory!=IntPtr.Zero)Marshal.FreeHGlobal(memory);if(service!=IntPtr.Zero)CloseServiceHandle(service);CloseServiceHandle(manager);}
  }
 }
}
'@}

function Inspect-Service {return [CodexProxyTray.ServiceQaNative]::Inspect($serviceName)}
function Assert-FileAcl([string]$Path) {
    No-Reparse $Path
    $acl=Get-Acl -LiteralPath $Path
    Require ($acl.GetOwner([Security.Principal.SecurityIdentifier]).Value -in @('S-1-5-18','S-1-5-32-544')) 'file_owner_not_trusted'
    Require $acl.AreAccessRulesProtected 'file_acl_not_protected'
    $rules=@($acl.GetAccessRules($true,$true,[Security.Principal.SecurityIdentifier]));Require ($rules.Count -eq 3) 'file_acl_unknown_rules'
    $seen=[Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach($rule in $rules){
        $sid=$rule.IdentityReference.Value
        Require ($rule.AccessControlType -eq [Security.AccessControl.AccessControlType]::Allow -and -not $rule.IsInherited) 'file_acl_unexpected_rule_type'
        Require ($seen.Add($sid)) 'file_acl_duplicate_rule'
        $expected=if($sid -in @('S-1-5-18','S-1-5-32-544')){0x1f01ff}elseif($sid -eq 'S-1-5-32-545'){0x1200a9}else{throw 'file_acl_unknown_principal'}
        Require ([int64]$rule.FileSystemRights -eq $expected) 'file_acl_rights_invalid'
    }
}
function Assert-ServiceConfig($Info) {
    Require ($null -ne $Info) 'service_not_registered'
    Require ($Info.Type -eq 0x10 -and $Info.StartType -eq 3 -and $Info.ErrorControl -eq 1) 'service_type_or_start_invalid'
    Require ([string]::Equals($Info.Account,'LocalSystem',[StringComparison]::OrdinalIgnoreCase)) 'service_account_invalid'
    Require ($Info.Display -eq $displayName -and $Info.Binary -ceq $expectedCommand -and $Info.NoDependencies -and $Info.NoLoadGroup) 'service_config_not_owned'
    $security=[Security.AccessControl.RawSecurityDescriptor]::new($Info.Security,0)
    Require ($security.Owner.Value -in @('S-1-5-18','S-1-5-32-544')) 'service_owner_not_trusted'
    Require ($null -ne $security.DiscretionaryAcl -and $security.DiscretionaryAcl.Count -eq 3) 'service_acl_unknown_rules'
    $seen=[Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach($ace in $security.DiscretionaryAcl){
        Require ($ace -is [Security.AccessControl.CommonAce] -and $ace.AceQualifier -eq [Security.AccessControl.AceQualifier]::AccessAllowed) 'service_acl_unexpected_rule_type'
        $sid=$ace.SecurityIdentifier.Value;Require ($seen.Add($sid)) 'service_acl_duplicate_rule'
        $expected=if($sid -in @('S-1-5-18','S-1-5-32-544')){0xf01ff}elseif($sid -eq $ownerSid){0x20015}else{throw 'service_acl_unknown_principal'}
        Require ($ace.AccessMask -eq $expected) 'service_acl_rights_invalid'
    }
}
function Assert-ProcessAcl([uint32]$ProcessId) {
    $descriptor=[Security.AccessControl.RawSecurityDescriptor]::new([CodexProxyTray.ServiceQaNative]::ProcessSecurity($ProcessId),0)
    Require ($null -ne $descriptor.DiscretionaryAcl -and $descriptor.DiscretionaryAcl.Count -eq 3) 'process_acl_unknown_rules'
    $seen=[Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach($ace in $descriptor.DiscretionaryAcl){
        Require ($ace -is [Security.AccessControl.CommonAce] -and $ace.AceType -eq [Security.AccessControl.AceType]::AccessAllowed -and -not ($ace.AceFlags -band [Security.AccessControl.AceFlags]::Inherited)) 'process_acl_unexpected_rule_type'
        $sid=$ace.SecurityIdentifier.Value;Require ($seen.Add($sid)) 'process_acl_duplicate_rule'
        $expected=if($sid -in @('S-1-5-18','S-1-5-32-544')){0x1fffff}elseif($sid -eq $ownerSid){0x101000}else{throw 'process_acl_unknown_principal'}
        Require ($ace.AccessMask -eq $expected) 'process_acl_rights_invalid'
    }
}
function Assert-OwnedInstall([switch]$AllowPrepared,[switch]$AllowMissingService) {
    Require (Test-Path -LiteralPath $serviceDirectory -PathType Container) 'owned_directory_missing'
    Require (Same-Path ((Get-Item -LiteralPath $serviceDirectory -Force).FullName) $serviceDirectory) 'owned_directory_changed'
    Assert-FileAcl $productDirectory;Assert-FileAcl $serviceDirectory
    $entries=@(Get-ChildItem -LiteralPath $serviceDirectory -Force)
    Require ($entries.Count -eq 3) 'owned_inventory_incomplete_or_unknown'
    foreach($entry in $entries){Require (-not $entry.PSIsContainer -and $entry.Name -in @($imageName,$loaderName,'receipt.json')) 'unknown_install_file_preserved';Assert-FileAcl $entry.FullName}
    Require ((Get-FileHash -LiteralPath $serviceImage -Algorithm SHA256).Hash.ToLowerInvariant() -eq $binaryHash) 'installed_image_hash_mismatch'
    Require ((Get-FileHash -LiteralPath (Join-Path $serviceDirectory $loaderName) -Algorithm SHA256).Hash.ToLowerInvariant() -eq $loaderHash) 'installed_loader_hash_mismatch'
    $receiptPath=Join-Path $serviceDirectory 'receipt.json';Require ((Get-Item -LiteralPath $receiptPath).Length -le 16384) 'receipt_too_large'
    $receipt=Get-Content -LiteralPath $receiptPath -Raw -Encoding utf8|ConvertFrom-Json
    $keys=@('schema','product','owner_sid','version','sha256','loader_sha256','image','service_command','phase','readiness','previous','created_parent')
    foreach($key in $keys){Require ($key -in $receipt.PSObject.Properties.Name) 'receipt_schema_missing_field'}
    Require ($receipt.schema -eq 1 -and $receipt.product -eq 'CodexProxyTray.TrafficService' -and $receipt.owner_sid -ceq $ownerSid) 'receipt_owner_or_product_invalid'
    Require ($receipt.version -match '^\d+\.\d+\.\d+$' -and $receipt.sha256 -ceq $binaryHash -and $receipt.loader_sha256 -ceq $loaderHash) 'receipt_version_or_hash_invalid'
    Require ($receipt.image -ceq $serviceImage -and $receipt.service_command -ceq $expectedCommand -and $null -eq $receipt.previous) 'receipt_paths_or_previous_invalid'
    Require ($receipt.created_parent -is [bool] -and $receipt.created_parent -eq (-not $parentInitiallyExisted)) 'receipt_parent_ownership_invalid'
    $installed=$receipt.phase -eq 'installed' -and $receipt.readiness -eq 'scm_configuration_verified'
    $prepared=$AllowPrepared -and $receipt.phase -eq 'prepared' -and $receipt.readiness -eq 'files_verified'
    Require ($installed -or $prepared) 'receipt_not_committed'
    $info=Inspect-Service
    if($null -ne $info){Assert-ServiceConfig $info}elseif(-not $AllowMissingService){throw 'service_not_registered'}
    return [pscustomobject]@{receipt_hash=(Get-FileHash -LiteralPath $receiptPath -Algorithm SHA256).Hash.ToLowerInvariant();version=$receipt.version;service=$info}
}
function Stop-OwnedInstaller([Diagnostics.Process]$Process,[datetime]$Created) {
    if($Process.HasExited){return}
    Require ($Process.StartTime.ToUniversalTime() -eq $Created -and (Same-Path $Process.Path $binary)) 'installer_identity_changed_not_killed'
    # Installer CLI has no descendants. Never kill by name, and never terminate the SCM service process.
    $Process.Kill($false);Require ($Process.WaitForExit(5000)) 'owned_installer_did_not_exit'
}
function Run-Installer([ValidateSet('--install-traffic-service','--uninstall-traffic-service')][string]$Action) {
    Require ((Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant() -eq $binaryHash) 'source_binary_changed'
    Require ((Get-FileHash -LiteralPath $sourceLoader -Algorithm SHA256).Hash.ToLowerInvariant() -eq $loaderHash) 'source_loader_changed'
    $process=$null;$created=$null;$exit=$null
    $script:diagnosticSequence++
    $stderr=Join-Path $diagnosticRoot ('installer-'+$diagnosticSequence+'.stderr')
    $stdout=Join-Path $diagnosticRoot ('installer-'+$diagnosticSequence+'.stdout')
    [void]$diagnosticFiles.Add($stderr);[void]$diagnosticFiles.Add($stdout)
    Require (-not (Test-Path -LiteralPath $stderr) -and -not (Test-Path -LiteralPath $stdout)) 'diagnostic_file_collision'
    try {
        $process=Start-Process -FilePath $binary -ArgumentList @($Action,'--owner-sid',$ownerSid) -WorkingDirectory ([IO.Path]::GetDirectoryName($binary)) -WindowStyle Hidden -PassThru -RedirectStandardError $stderr -RedirectStandardOutput $stdout -ErrorAction Stop -Verbose:$false -Debug:$false
        $created=$process.StartTime.ToUniversalTime()
        if(-not $process.WaitForExit($ProcessTimeoutSeconds*1000)){$script:writeUncertain=$true;Stop-OwnedInstaller $process $created;throw 'installer_timeout_preserved'}
        $exit=$process.ExitCode;$report.installer_exit_codes.Add($exit)
        if($exit -eq 3){$script:writeUncertain=$true}
        Require ($exit -eq 0) ('installer_exit_'+$exit)
    } finally {
        if($process){try{if(-not $process.HasExited){$script:writeUncertain=$true;if($null -ne $created){Stop-OwnedInstaller $process $created}}}finally{$process.Dispose()}}
        if($null -eq $exit -or $exit -ne 0){$report.installer_diagnostics.Add([ordered]@{action=if($Action -eq '--install-traffic-service'){'install'}else{'uninstall'};exit_code=$exit;stderr_short=(Short-Diagnostic $stderr)})}
    }
}
function Stop-OwnedService {
    $owned=Assert-OwnedInstall;$info=$owned.service
    if($info.State -eq 1){return}
    $deadline=[DateTime]::UtcNow.AddSeconds($ServiceTimeoutSeconds)
    while($info.State -eq 4 -and -not($info.Controls -band 1)){Require ([DateTime]::UtcNow -lt $deadline) 'service_stop_control_not_ready';Start-Sleep -Milliseconds 50;$info=(Assert-OwnedInstall).service}
    $controller=Get-Service -Name $serviceName -ErrorAction Stop
    try {
        if($info.State -ne 3){Require (-not $script:stopSent) 'stop_already_requested';$script:stopSent=$true;Stop-Service -Name $serviceName -NoWait -ErrorAction Stop -WarningAction Stop -Confirm:$false}
        $controller.WaitForStatus([ServiceProcess.ServiceControllerStatus]::Stopped,[TimeSpan]::FromSeconds($ServiceTimeoutSeconds))
    } finally {$controller.Dispose()}
    Require ((Inspect-Service).State -eq 1) 'service_stop_not_confirmed'
    if($script:serviceProcess){Require ($script:serviceProcess.WaitForExit($ServiceTimeoutSeconds*1000)) 'service_process_exit_not_confirmed'}
}
function Assert-Removed {
    Require ($null -eq (Inspect-Service)) 'service_registration_remains'
    Require (-not (Test-Path -LiteralPath $serviceDirectory)) 'service_directory_remains'
    if($parentInitiallyExisted){Require (Test-Path -LiteralPath $productDirectory -PathType Container) 'preexisting_parent_not_preserved'}else{Require (-not (Test-Path -LiteralPath $productDirectory)) 'created_parent_directory_remains'}
}
function Run-Case([string]$Name,[scriptblock]$Check) {
    $script:currentCase=$Name;$clock=[Diagnostics.Stopwatch]::StartNew()
    try {$null=&$Check;$cases.Add([ordered]@{name=$Name;status='passed';milliseconds=$clock.ElapsedMilliseconds})}
    catch {$cases.Add([ordered]@{name=$Name;status='failed';milliseconds=$clock.ElapsedMilliseconds;code=(Failure-Code $_)});throw}
}
function Run-ServicePipeline {
    $sandbox=Join-Path $diagnosticRoot 'pipeline-data'
    Require (-not (Test-Path -LiteralPath $sandbox)) 'pipeline_sandbox_exists'
    [void][IO.Directory]::CreateDirectory((Join-Path $sandbox 'CodexProxyTray'))
    [IO.File]::WriteAllText((Join-Path $sandbox 'CodexProxyTray/config.json'),'{"launch_target":"none","network_enabled":false,"connectivity_enabled":false,"traffic_enabled":true,"auto_check_updates":false}')
    $start={param([string]$Arguments)
        $info=[Diagnostics.ProcessStartInfo]::new();$info.FileName=$binary;$info.Arguments=$Arguments;$info.WorkingDirectory=[IO.Path]::GetDirectoryName($binary);$info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.WindowStyle='Hidden'
        $info.EnvironmentVariables['LOCALAPPDATA']=$sandbox;$info.EnvironmentVariables['WEBVIEW2_USER_DATA_FOLDER']=Join-Path $sandbox 'webview-data'
        return [Diagnostics.Process]::Start($info)
    }
    $gui=$null;$pipelineFailure=$null;$final=$null
    try{
        Require (@(Get-Process -Name CodexProxyTray -ErrorAction SilentlyContinue).Count -eq 0) 'pipeline_other_gui_preserved'
        $gui=&$start '--background';$guiStarted=$gui.StartTime.ToUniversalTime();$deadline=[DateTime]::UtcNow.AddSeconds(40);$seen=[Collections.Generic.HashSet[long]]::new()
        while([DateTime]::UtcNow -lt $deadline){
            Require (-not $gui.HasExited) 'pipeline_gui_exited'
            $path=Join-Path $sandbox 'CodexProxyTray/status.json'
            if(Test-Path -LiteralPath $path){
                try{$value=Get-Content -LiteralPath $path -Raw|ConvertFrom-Json}catch{Start-Sleep -Milliseconds 100;continue}
                if($value.helper_pid -eq $gui.Id){
                    if($value.traffic.state -in @('unavailable','needs_permission')){$report['pipeline_error_message']=[string]$value.traffic.message;throw 'service_pipeline_unavailable'}
                    if($value.traffic.state -eq 'collecting' -and $null -ne $value.traffic.received_bytes -and $null -ne $value.traffic.sent_bytes){[void]$seen.Add([long]$value.traffic.timestamp_ms)}
                    if($seen.Count -ge 2){break}
                }
            }
            Start-Sleep -Milliseconds 150
        }
        Require ($seen.Count -ge 2) 'service_pipeline_samples_missing'
        $report['service_pipeline_samples']=$seen.Count
    }catch{$pipelineFailure=$_}
    finally{
        if($gui){
            try{
                if(-not $gui.HasExited){Require ($gui.StartTime.ToUniversalTime() -eq $guiStarted -and (Same-Path $gui.Path $binary)) 'pipeline_gui_identity_changed';$quitter=&$start '--quit';try{Require ($quitter.WaitForExit(10000)) 'pipeline_quit_request_timeout'}finally{$quitter.Dispose()};Require ($gui.WaitForExit(15000)) 'pipeline_gui_stop_timeout'}
                $statusPath=Join-Path $sandbox 'CodexProxyTray/status.json';if(Test-Path -LiteralPath $statusPath){$final=Get-Content -LiteralPath $statusPath -Raw|ConvertFrom-Json}
            }catch{if(-not $gui.HasExited -and $gui.StartTime.ToUniversalTime() -eq $guiStarted -and (Same-Path $gui.Path $binary)){$gui.Kill($true);[void]$gui.WaitForExit(10000)};if(-not $pipelineFailure){$pipelineFailure=$_}}
            finally{$gui.Dispose()}
        }
        $until=[DateTime]::UtcNow.AddSeconds(12)
        do{$collectors=@(Get-CimInstance Win32_Process -Filter "Name = 'CodexProxyTrayTrafficService.exe'"|Where-Object {$_.CommandLine -like '*--traffic-collector*' -and (Same-Path $_.ExecutablePath $serviceImage)});if($collectors.Count){Start-Sleep -Milliseconds 100}}while($collectors.Count -and [DateTime]::UtcNow -lt $until)
        if($collectors.Count -and -not $pipelineFailure){$pipelineFailure='pipeline_collectors_remain'}
        try{No-Reparse $sandbox;Require (Is-Within (Full-Path $sandbox) $diagnosticRoot) 'pipeline_cleanup_boundary';Start-Sleep -Seconds 2;Remove-Item -LiteralPath $sandbox -Recurse -Force}catch{if(-not $pipelineFailure){$pipelineFailure=$_}}
    }
    if($pipelineFailure){throw $pipelineFailure}
    Require ($final.helper_pid -gt 0 -and -not $final.backend_ready -and $final.traffic.state -eq 'disabled') 'pipeline_stop_ack_not_confirmed'
    $report['service_pipeline_stop_ack']=$true;$report['service_pipeline_collectors_exited']=$true
}

$evidenceRoot=Check-EvidencePath $EvidenceDirectory
[void][IO.Directory]::CreateDirectory($evidenceRoot)
$diagnosticRoot=Check-EvidencePath ($evidenceRoot+'-work-'+[Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($diagnosticRoot)
$diagnosticOwned=$true;$diagnosticSequence=0;$diagnosticFiles=[Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
$cases=[Collections.Generic.List[object]]::new();$failure=$false;$currentCase='preflight';$installAttempted=$false;$uninstallAttempted=$false;$writeUncertain=$false;$stopSent=$false;$serviceProcess=$null
$report=[ordered]@{format='codex-proxy-tray-traffic-service-qa-v1';status='running';started_at=[DateTime]::UtcNow.ToString('o');scope='Fresh GitHub-hosted runner; protected install, SCM broker, real service-backed ETW samples and STOP acknowledgement; no UI automation, screenshots, external probes or real accounts';installer_exit_codes=[Collections.Generic.List[int]]::new();installer_diagnostics=[Collections.Generic.List[object]]::new();cases=$cases;cleanup='not_needed';failure_case=$null;failure_code=$null;tests=$null}
try {
    Run-Case 'isolated_elevated_preflight' {
        Require ([Environment]::Is64BitProcess -and [CodexProxyTray.ServiceQaNative]::Elevated()) 'elevated_x64_process_required'
        $script:ownerSid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
        Require ($ownerSid -match '^S-1-' -and $ownerSid -ne 'S-1-5-18') 'installer_owner_invalid'
        $script:binary=(Resolve-Path -LiteralPath $BinaryPath).Path
        Require ([IO.Path]::GetFileName($binary) -ceq 'CodexProxyTray.exe' -and (Test-Path -LiteralPath $binary -PathType Leaf)) 'compiled_gui_binary_required'
        No-Reparse $binary
        Require ((Get-Item -LiteralPath $binary).Length -le 128MB) 'source_binary_too_large'
        $script:sourceLoader=Join-Path ([IO.Path]::GetDirectoryName($binary)) $loaderName
        Require ((Get-FileHash -LiteralPath $sourceLoader -Algorithm SHA256).Hash.ToLowerInvariant() -eq $loaderHash) 'fixed_loader_required'
        $script:binaryHash=(Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant();$report.binary_sha256=$binaryHash;$report.loader_sha256=$loaderHash
        $known=Full-Path ([CodexProxyTray.ServiceQaNative]::ProgramFiles());No-Reparse $known
        $script:productDirectory=Join-Path $known 'CodexProxyTray';$script:serviceDirectory=Join-Path $productDirectory 'TrafficService';$script:serviceImage=Join-Path $serviceDirectory $imageName
        Require (Is-Within $serviceDirectory $known) 'service_directory_escaped_known_folder'
        $script:expectedCommand='"'+$serviceImage+'" --traffic-service'
        Require ($null -eq (Inspect-Service) -and -not (Test-Path -LiteralPath $serviceDirectory)) 'preexisting_service_or_directory_preserved'
        $script:parentInitiallyExisted=Test-Path -LiteralPath $productDirectory
        if($parentInitiallyExisted){Assert-FileAcl $productDirectory}
        $report.preflight_absent=$true;$report.token_elevated=$true
    }
    Run-Case 'install_and_verify_protected_receipt_scm' {
        $script:installAttempted=$true;Run-Installer '--install-traffic-service'
        $verified=Assert-OwnedInstall;$script:firstReceipt=$verified.receipt_hash;$report.receipt_sha256=$verified.receipt_hash;$report.version=$verified.version
        $report.protected_inventory_verified=$true;$report.owner_and_acl_verified=$true;$report.receipt_verified=$true;$report.scm_configuration_verified=$true
        Require ($verified.service.State -eq 1) 'install_unexpectedly_started_service'
    }
    Run-Case 'same_hash_install_is_idempotent' {
        Run-Installer '--install-traffic-service';$verified=Assert-OwnedInstall
        Require ($verified.receipt_hash -ceq $firstReceipt -and $verified.service.State -eq 1) 'idempotent_install_changed_receipt_or_state'
        $report.idempotent_install_verified=$true
    }
    Run-Case 'scm_starts_real_service_main' {
        $null=Assert-OwnedInstall
        Start-Service -Name $serviceName -ErrorAction Stop -WarningAction Stop
        $controller=Get-Service -Name $serviceName -ErrorAction Stop
        try {$controller.WaitForStatus([ServiceProcess.ServiceControllerStatus]::Running,[TimeSpan]::FromSeconds($ServiceTimeoutSeconds))}finally{$controller.Dispose()}
        $verified=Assert-OwnedInstall;Require ($verified.service.State -eq 4 -and $verified.service.ProcessId -gt 0) 'service_main_not_running'
        $script:serviceProcess=Get-Process -Id $verified.service.ProcessId -ErrorAction Stop
        Require (Same-Path $serviceProcess.Path $serviceImage) 'running_service_image_mismatch'
        Assert-ProcessAcl $verified.service.ProcessId
        $report.process_acl_verified=$true
        $report.real_service_main_running=$true
    }
    Run-Case 'service_backed_sampling_roundtrip_and_stop_ack' {Run-ServicePipeline}
    Run-Case 'stop_only_owned_service' {Stop-OwnedService;$report.service_stop_verified=$true}
    Run-Case 'uninstall_removes_registration_and_owned_directory' {
        $null=Assert-OwnedInstall;$script:uninstallAttempted=$true;Run-Installer '--uninstall-traffic-service';Assert-Removed;$report.uninstall_verified=$true
    }
    $report.status='passed'
} catch {$failure=$true;$report.status='failed';$report.failure_case=$currentCase;$report.failure_code=Failure-Code $_}
finally {
    try {
        if($installAttempted){
            $present=($null -ne (Inspect-Service)) -or (Test-Path -LiteralPath $serviceDirectory)
            if(-not $present){Assert-Removed;$report.cleanup='passed'}
            elseif($writeUncertain -or $uninstallAttempted){$report.cleanup='blocked';$failure=$true;$report.status='failed'}
            else {
                # Revalidate every owned object before the single reverse operation; unknown/recovery files are preserved.
                $null=Assert-OwnedInstall -AllowPrepared -AllowMissingService
                if($null -ne (Inspect-Service)){Stop-OwnedService}
                $script:uninstallAttempted=$true;Run-Installer '--uninstall-traffic-service';Assert-Removed;$report.cleanup='passed'
            }
        }
    } catch {$report.cleanup='blocked';$report.cleanup_code=Failure-Code $_;$failure=$true;$report.status='failed'}
    try {Remove-Diagnostics;$report.diagnostic_cleanup='passed'}catch{$report.diagnostic_cleanup='blocked';$report.diagnostic_cleanup_code=Failure-Code $_;$failure=$true;$report.status='failed'}
    if($failure){try{$state=Inspect-Service;if($state){$report.retained_service_state=$state.State;$report.retained_service_win32_exit=$state.Win32Exit;$report.retained_service_specific_exit=$state.SpecificExit}}catch{}}
    if($serviceProcess){$serviceProcess.Dispose()}
    $report.tests=[ordered]@{pass=@($cases|Where-Object status -eq 'passed').Count;fail=@($cases|Where-Object status -eq 'failed').Count}
    $report.finished_at=[DateTime]::UtcNow.ToString('o')
    [void](Check-EvidencePath $evidenceRoot -AllowExisting)
    Write-NewJson (Join-Path $evidenceRoot 'service-checks.json') $report
}
if($failure){throw 'Traffic service QA failed; inspect sanitized service-checks.json. Unknown or uncertain objects were preserved.'}
Write-Output ('Traffic service QA passed: '+$cases.Count+' cases; owned service and directory removed.')
