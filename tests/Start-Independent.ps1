[CmdletBinding()]
param(
    [string]$Executable,
    [switch]$Background,
    [ValidateSet('Show','Refresh','Quit')][string]$Action='Show',
    [string]$CheckScript,
    [string]$EvidenceDirectory,
    [string]$BaselinePath,
    [switch]$CaptureBaseline,
    [ValidateRange(10,300)][int]$TimeoutSeconds=90
)
$ErrorActionPreference='Stop'
function Quote-Argument([string]$Value) {
    if ($Value.IndexOf('"') -ge 0 -or $Value.ToCharArray().Where({[char]::IsControl($_)}).Count) { throw 'A path contains unsupported quotes or control characters.' }
    # Native Windows argument parsing requires trailing backslashes to be doubled before the closing quote.
    return '"' + [regex]::Replace($Value, '(\\+)$', '$1$1') + '"'
}
function FileSystem-Path([string]$Value) {
    $provider=$null; $drive=$null
    $path=$ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Value,[ref]$provider,[ref]$drive)
    if ($provider.Name -ne 'FileSystem') { throw 'Choose a filesystem path.' }
    return [IO.Path]::GetFullPath($path)
}
function Stop-OwnedCheck([Diagnostics.Process]$Process) {
    if ($Process.HasExited) { return }
    if ($Process.GetType().GetMethod('Kill', [type[]]@([bool]))) { $Process.Kill($true) }
    else {
        $start=New-Object Diagnostics.ProcessStartInfo
        $start.FileName=Join-Path $env:SystemRoot 'System32\taskkill.exe'
        $start.Arguments='/PID ' + $Process.Id + ' /T /F'
        $start.UseShellExecute=$false; $start.CreateNoWindow=$true; $start.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
        $killer=[Diagnostics.Process]::Start($start)
        try {
            if (-not $killer.WaitForExit(10000)) { $killer.Kill(); throw 'The owned check could not be stopped within the timeout.' }
            if ($killer.ExitCode -ne 0 -and -not $Process.HasExited) { throw 'The owned check process tree could not be stopped.' }
        } finally { $killer.Dispose() }
    }
    if (-not $Process.WaitForExit(5000)) { throw 'The owned check process has not exited after termination.' }
}
if ($CheckScript) {
    if ($Executable -or $Background -or $Action -ne 'Show') { throw 'CheckScript cannot be combined with application launch options.' }
    $CheckScript=(Resolve-Path -LiteralPath $CheckScript).Path
    if (-not (Test-Path -LiteralPath $CheckScript -PathType Leaf) -or [IO.Path]::GetDirectoryName($CheckScript) -ne $PSScriptRoot -or [IO.Path]::GetFileName($CheckScript) -notlike 'Test-*.ps1') { throw 'Choose a repository-owned Test-*.ps1 script from this tests directory.' }
    $Executable=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $arguments='-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File ' + (Quote-Argument $CheckScript)
    if ($CaptureBaseline) {
        if (-not $BaselinePath -or $EvidenceDirectory) { throw 'Baseline capture requires BaselinePath and does not accept EvidenceDirectory.' }
        $arguments+=' -CaptureBaseline -BaselinePath ' + (Quote-Argument (FileSystem-Path $BaselinePath))
    } else {
        if (-not $EvidenceDirectory) { throw '-EvidenceDirectory is required for script checks.' }
        $arguments+=' -EvidenceDirectory ' + (Quote-Argument (FileSystem-Path $EvidenceDirectory))
        if ($BaselinePath) { $arguments+=' -BaselinePath ' + (Quote-Argument (Resolve-Path -LiteralPath $BaselinePath).Path) }
    }
} else {
    if (-not $Executable -or $CaptureBaseline -or $EvidenceDirectory -or $BaselinePath) { throw 'Select an executable, or use CheckScript with its check parameters.' }
    $Executable=(Resolve-Path -LiteralPath $Executable).Path
    if (-not (Test-Path -LiteralPath $Executable -PathType Leaf) -or [IO.Path]::GetFileName($Executable) -ne 'CodexProxyTray.exe') { throw 'Select the built CodexProxyTray executable.' }
    $arguments=if($Action -eq 'Quit'){'--quit'}elseif($Action -eq 'Refresh'){'--refresh'}elseif($Background){'--background'}else{''}
}
if (-not ('CodexProxyTray.IndependentLaunch' -as [type])) {
Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Security.Principal;
using System.Text;
namespace CodexProxyTray {
 public static class IndependentLaunch {
  [StructLayout(LayoutKind.Sequential,CharSet=CharSet.Unicode)] struct StartupInfo { public int cb; public string reserved,desktop,title; public int x,y,xsize,ysize,xchars,ychars,fill,flags; public short show,reservedSize; public IntPtr reservedPtr,input,output,error; }
  [StructLayout(LayoutKind.Sequential)] struct StartupInfoEx { public StartupInfo start; public IntPtr attributes; }
  [StructLayout(LayoutKind.Sequential)] struct ProcessInfo { public IntPtr process,thread; public int pid,tid; }
  [DllImport("kernel32.dll",SetLastError=true)] static extern IntPtr OpenProcess(uint access,bool inherit,int pid);
  [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool InitializeProcThreadAttributeList(IntPtr list,int count,int flags,ref IntPtr size);
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool UpdateProcThreadAttribute(IntPtr list,uint flags,IntPtr attribute,IntPtr value,IntPtr size,IntPtr previous,IntPtr returned);
  [DllImport("kernel32.dll")] static extern void DeleteProcThreadAttributeList(IntPtr list);
  [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool CreateProcessW(string app,StringBuilder args,IntPtr ps,IntPtr ts,bool inherit,uint flags,IntPtr environment,string directory,ref StartupInfoEx startup,out ProcessInfo result);
  [DllImport("kernel32.dll",CharSet=CharSet.Unicode)] static extern int GetPackageFamilyName(IntPtr process,ref uint length,StringBuilder name);
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool IsProcessInJob(IntPtr process,IntPtr job,out bool inside);
  [DllImport("kernel32.dll",SetLastError=true)] static extern uint ResumeThread(IntPtr thread);
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool TerminateProcess(IntPtr process,uint code);
  [DllImport("userenv.dll",SetLastError=true)] static extern bool CreateEnvironmentBlock(out IntPtr environment,IntPtr token,bool inherit);
  [DllImport("userenv.dll")] static extern bool DestroyEnvironmentBlock(IntPtr environment);
  [DllImport("advapi32.dll",SetLastError=true)] static extern bool OpenProcessToken(IntPtr process,uint access,out IntPtr token);
  public static Process Start(string exe,int explorer,string arguments,bool hidden) {
   IntPtr parent=OpenProcess(0x1080,false,explorer);if(parent==IntPtr.Zero)throw new Win32Exception();
   IntPtr size=IntPtr.Zero,list=IntPtr.Zero,value=IntPtr.Zero,environment=IntPtr.Zero,desktopToken=IntPtr.Zero;bool initialized=false,created=false,resumed=false;
   ProcessInfo result=new ProcessInfo();Process retained=null;
   try {
    InitializeProcThreadAttributeList(IntPtr.Zero,1,0,ref size);list=Marshal.AllocHGlobal(size);value=Marshal.AllocHGlobal(IntPtr.Size);Marshal.WriteIntPtr(value,parent);
    if(!InitializeProcThreadAttributeList(list,1,0,ref size))throw new Win32Exception();initialized=true;
    if(!UpdateProcThreadAttribute(list,0,new IntPtr(0x20000),value,new IntPtr(IntPtr.Size),IntPtr.Zero,IntPtr.Zero))throw new Win32Exception();
    if(!OpenProcessToken(parent,8,out desktopToken))throw new Win32Exception();
    using(var current=WindowsIdentity.GetCurrent())using(var desktop=new WindowsIdentity(desktopToken)) {if(current.User==null||desktop.User==null||!current.User.Equals(desktop.User))throw new InvalidOperationException("The Explorer process belongs to another user.");if(!CreateEnvironmentBlock(out environment,desktopToken,false))throw new Win32Exception();}
    var startup=new StartupInfoEx();startup.start.cb=Marshal.SizeOf(typeof(StartupInfoEx));startup.attributes=list;
    if(hidden){startup.start.flags=1;startup.start.show=0;}
    uint flags=0x00080000|0x00000400|0x00000004|(hidden?0x08000000u:0);
    if(!CreateProcessW(exe,new StringBuilder("\""+exe+"\" "+arguments),IntPtr.Zero,IntPtr.Zero,false,flags,environment,System.IO.Path.GetDirectoryName(exe),ref startup,out result))throw new Win32Exception();created=true;
    uint length=512;var family=new StringBuilder((int)length);int packageStatus=GetPackageFamilyName(result.process,ref length,family);
    if(packageStatus!=15700)throw new InvalidOperationException("The new process is not unpackaged; package status="+packageStatus+". It has not been resumed.");
    retained=Process.GetProcessById(result.pid);GC.KeepAlive(retained.Handle);
    if(ResumeThread(result.thread)==UInt32.MaxValue)throw new Win32Exception();resumed=true;return retained;
   } catch {if(created&&!resumed)TerminateProcess(result.process,1);if(retained!=null)retained.Dispose();throw;}
   finally{if(result.thread!=IntPtr.Zero)CloseHandle(result.thread);if(result.process!=IntPtr.Zero)CloseHandle(result.process);if(environment!=IntPtr.Zero)DestroyEnvironmentBlock(environment);if(desktopToken!=IntPtr.Zero)CloseHandle(desktopToken);if(initialized)DeleteProcThreadAttributeList(list);if(list!=IntPtr.Zero)Marshal.FreeHGlobal(list);if(value!=IntPtr.Zero)Marshal.FreeHGlobal(value);CloseHandle(parent);}
  }
  public static string Identity(Process process) {
   uint size=512;var name=new StringBuilder((int)size);int status=GetPackageFamilyName(process.Handle,ref size,name);bool job;
   if(!IsProcessInJob(process.Handle,IntPtr.Zero,out job))throw new Win32Exception();
   return "package_status="+status+";package="+name+";in_job="+job;
  }
 }
}
'@
}
$session=(Get-Process -Id $PID).SessionId
$explorers=@(Get-Process -Name explorer | Where-Object {$_.SessionId -eq $session})
if ($explorers.Count -ne 1) { throw 'Expected one current-session Explorer desktop process; no process was started.' }
$launched=[CodexProxyTray.IndependentLaunch]::Start($Executable,$explorers[0].Id,$arguments,[bool]$CheckScript)
try {
    if ($CheckScript) {
        $identity=[CodexProxyTray.IndependentLaunch]::Identity($launched)
        $deadline=[DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
        while (-not $launched.WaitForExit(1000)) {if([DateTime]::UtcNow -ge $deadline){Stop-OwnedCheck $launched;throw 'The owned native check timed out and was stopped.'}}
        [pscustomobject]@{check_pid=$launched.Id;desktop_parent_pid=$explorers[0].Id;identity=$identity;exit_code=$launched.ExitCode} | ConvertTo-Json
        if ($launched.ExitCode -ne 0) { throw 'The independent native check failed; inspect its evidence JSON.' }
    } else {
        Start-Sleep -Milliseconds 700
        [pscustomobject]@{launch_pid=$launched.Id;desktop_parent_pid=$explorers[0].Id;identity=$(if(-not $launched.HasExited){[CodexProxyTray.IndependentLaunch]::Identity($launched)}else{$null});launcher_exited=$launched.HasExited} | ConvertTo-Json
    }
} finally {try {if ($CheckScript -and -not $launched.HasExited) {Stop-OwnedCheck $launched}} finally {$launched.Dispose()}}
