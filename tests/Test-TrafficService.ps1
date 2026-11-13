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
    while($exception){if($exception.Message -cmatch '^[a-z][a-z0-9_]{1,90}$'){return $exception.Message};if($exception -is [ComponentModel.Win32Exception]){return 'win32_error_'+$exception.NativeErrorCode};$exception=$exception.InnerException}
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
    $pipeline=Join-Path $diagnosticRoot 'pipeline-data'
    if(Test-Path -LiteralPath $pipeline){No-Reparse $pipeline;Require ($report.status -eq 'failed') 'pipeline_directory_after_success';return 'retained_for_debugging'}
    $entries=@(Get-ChildItem -LiteralPath $diagnosticRoot -Force)
    foreach($entry in $entries){Require (-not $entry.PSIsContainer -and $diagnosticFiles.Contains($entry.FullName)) 'unknown_diagnostic_file_preserved';No-Reparse $entry.FullName}
    foreach($entry in $entries){[IO.File]::Delete($entry.FullName)}
    [IO.Directory]::Delete($diagnosticRoot,$false)
    return 'passed'
}

if(-not ('CodexProxyTray.ServiceQaNative' -as [type])) {Add-Type -TypeDefinition @'
using System;
using System.Collections;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Principal;
using System.Text;
using System.Threading;
namespace CodexProxyTray {
 public sealed class ServiceQaInfo {
  public uint Type,StartType,ErrorControl,State,ProcessId,Win32Exit,SpecificExit,Controls;
  public string Binary,Account,Display;
  public bool NoDependencies,NoLoadGroup;
  public byte[] Security;
 }
 public sealed class ServiceQaTokenInfo {
  public bool Elevated,AdminEnabled;
  public uint IntegrityRid;
  public string UserSid;
 }
 public static class ServiceQaNative {
  [StructLayout(LayoutKind.Sequential)] struct Config {public uint type,start,error;public IntPtr binary,group;public uint tag;public IntPtr dependencies,account,display;}
  [StructLayout(LayoutKind.Sequential)] struct Status {public uint type,state,controls,exit,specific,checkpoint,hint,pid,flags;}
  [StructLayout(LayoutKind.Sequential)] struct SidAndAttributes {public IntPtr Sid;public uint Attributes;}
  [StructLayout(LayoutKind.Sequential)] struct TokenGroupsOne {public uint Count;public SidAndAttributes First;}
  [StructLayout(LayoutKind.Sequential)] struct MandatoryLabel {public SidAndAttributes Label;}
  [StructLayout(LayoutKind.Sequential,CharSet=CharSet.Unicode)] struct StartupInfo {public uint cb;public string reserved,desktop,title;public uint x,y,xSize,ySize,xChars,yChars,fill,flags;public ushort show,reservedSize;public IntPtr reservedBytes,input,output,error;}
  [StructLayout(LayoutKind.Sequential)] struct ProcessInfo {public IntPtr process,thread;public uint pid,tid;}
  [StructLayout(LayoutKind.Sequential)] struct NativeFileTime {public uint low,high;public long Value {get{return ((long)high<<32)|low;}}}
  [StructLayout(LayoutKind.Sequential)] struct SecurityAttributes {public uint length;public IntPtr descriptor;public int inherit;}
  [StructLayout(LayoutKind.Sequential)] struct WnodeHeader {public uint size,provider;public ulong context;public long timestamp;public Guid guid;public uint clock,flags;}
  [StructLayout(LayoutKind.Sequential)] struct TraceProperties {public WnodeHeader node;public uint buffer,minBuffers,maxBuffers,maxFile,mode,flush,enable;public int age;public uint buffers,free,lost,written,logLost,realtimeLost;public IntPtr thread;public uint logName,loggerName;}
  [DllImport("shell32.dll",CharSet=CharSet.Unicode)] static extern int SHGetFolderPathW(IntPtr window,int folder,IntPtr token,uint flags,StringBuilder path);
  [DllImport("advapi32.dll",SetLastError=true)] static extern bool GetTokenInformation(IntPtr token,int kind,out uint information,uint length,out uint returned);
  [DllImport("advapi32.dll",EntryPoint="GetTokenInformation",SetLastError=true)] static extern bool GetTokenInformationBuffer(IntPtr token,int kind,IntPtr information,uint length,out uint returned);
  [DllImport("advapi32.dll",SetLastError=true)] static extern bool OpenProcessToken(IntPtr process,uint access,out IntPtr token);
  [DllImport("advapi32.dll",SetLastError=true)] static extern bool CreateRestrictedToken(IntPtr token,uint flags,uint disabledCount,[In] SidAndAttributes[] disabled,uint deletedCount,IntPtr deleted,uint restrictedCount,IntPtr restricted,out IntPtr result);
  [DllImport("advapi32.dll",SetLastError=true)] static extern bool SetTokenInformation(IntPtr token,int kind,ref MandatoryLabel information,uint length);
  [DllImport("advapi32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool ConvertStringSidToSidW(string text,out IntPtr sid);
  [DllImport("advapi32.dll")] static extern uint GetLengthSid(IntPtr sid);
  [DllImport("advapi32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool CreateProcessWithTokenW(IntPtr token,uint logon,string application,StringBuilder command,uint flags,IntPtr environment,string directory,ref StartupInfo startup,out ProcessInfo process);
  [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr memory);
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool GetProcessTimes(IntPtr process,out NativeFileTime created,out NativeFileTime exited,out NativeFileTime kernel,out NativeFileTime user);
  [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool QueryFullProcessImageNameW(IntPtr process,uint flags,StringBuilder path,ref uint size);
  [DllImport("kernel32.dll",SetLastError=true)] static extern uint ResumeThread(IntPtr thread);
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool TerminateProcess(IntPtr process,uint exitCode);
  [DllImport("kernel32.dll",SetLastError=true)] static extern uint WaitForSingleObject(IntPtr handle,uint milliseconds);
  [DllImport("kernel32.dll",SetLastError=true)] static extern uint GetProcessId(IntPtr process);
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool GetExitCodeProcess(IntPtr process,out uint code);
  [DllImport("advapi32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool ConvertStringSecurityDescriptorToSecurityDescriptorW(string text,uint revision,out IntPtr descriptor,out uint size);
  [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern IntPtr CreateNamedPipeW(string name,uint access,uint mode,uint instances,uint output,uint input,uint timeout,ref SecurityAttributes security);
  [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern IntPtr CreateFileW(string name,uint access,uint sharing,IntPtr security,uint disposition,uint flags,IntPtr template);
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool SetNamedPipeHandleState(IntPtr pipe,ref uint mode,IntPtr collection,IntPtr timeout);
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool ConnectNamedPipe(IntPtr pipe,IntPtr overlapped);
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool PeekNamedPipe(IntPtr pipe,IntPtr buffer,uint size,IntPtr read,out uint available,out uint remaining);
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool ReadFile(IntPtr file,[Out] byte[] data,uint size,out uint read,IntPtr overlapped);
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool WriteFile(IntPtr file,[In] byte[] data,uint size,out uint written,IntPtr overlapped);
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool GetNamedPipeClientProcessId(IntPtr pipe,out uint pid);
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool GetNamedPipeServerProcessId(IntPtr pipe,out uint pid);
  [DllImport("advapi32.dll",CharSet=CharSet.Unicode)] static extern uint ControlTraceW(ulong trace,string name,IntPtr properties,uint control);
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
  static IntPtr TokenBuffer(IntPtr token,int kind) {
   uint needed;GetTokenInformationBuffer(token,kind,IntPtr.Zero,0,out needed);if(needed==0||needed>65536)throw new InvalidOperationException("token_information_size");
   IntPtr memory=Marshal.AllocHGlobal((int)needed);if(!GetTokenInformationBuffer(token,kind,memory,needed,out needed)){Marshal.FreeHGlobal(memory);throw new Win32Exception();}return memory;
  }
  static ServiceQaTokenInfo ReadToken(IntPtr token) {
   uint elevated,returned;if(!GetTokenInformation(token,20,out elevated,4,out returned))throw new Win32Exception();
   var result=new ServiceQaTokenInfo {Elevated=elevated!=0};IntPtr memory=TokenBuffer(token,1);
   try {result.UserSid=new SecurityIdentifier(Marshal.ReadIntPtr(memory)).Value;}finally{Marshal.FreeHGlobal(memory);}
   memory=TokenBuffer(token,25);
   try {string sid=new SecurityIdentifier(Marshal.ReadIntPtr(memory)).Value;if(!sid.StartsWith("S-1-16-",StringComparison.Ordinal))throw new InvalidOperationException("integrity_sid_invalid");result.IntegrityRid=uint.Parse(sid.Substring(7));}finally{Marshal.FreeHGlobal(memory);}
   memory=TokenBuffer(token,2);
   try {int count=Marshal.ReadInt32(memory);if(count<0||count>4096)throw new InvalidOperationException("token_groups_size");int offset=(int)Marshal.OffsetOf(typeof(TokenGroupsOne),"First"),stride=Marshal.SizeOf(typeof(SidAndAttributes));for(int index=0;index<count;index++){var group=(SidAndAttributes)Marshal.PtrToStructure(IntPtr.Add(memory,offset+index*stride),typeof(SidAndAttributes));if(new SecurityIdentifier(group.Sid).Value=="S-1-5-32-544"&&(group.Attributes&4)!=0&&(group.Attributes&16)==0)result.AdminEnabled=true;}}
   finally {Marshal.FreeHGlobal(memory);}return result;
  }
  static ServiceQaTokenInfo ReadProcessToken(IntPtr process) {IntPtr token;if(!OpenProcessToken(process,8,out token))throw new Win32Exception();try{return ReadToken(token);}finally{CloseHandle(token);}}
  public static ServiceQaTokenInfo ProcessToken(uint pid) {IntPtr process=OpenProcess(0x1000,false,pid);if(process==IntPtr.Zero)throw new Win32Exception();try{return ReadProcessToken(process);}finally{CloseHandle(process);}}
  static IntPtr MediumToken() {
   IntPtr linked=IntPtr.Zero,restricted=IntPtr.Zero,admin=IntPtr.Zero,medium=IntPtr.Zero;
   try {using(var user=WindowsIdentity.GetCurrent()){
    IntPtr source=user.Token;uint elevationType,returned;if(!GetTokenInformation(source,18,out elevationType,4,out returned))throw new Win32Exception();
    if(elevationType==2){IntPtr memory=TokenBuffer(source,19);try{linked=Marshal.ReadIntPtr(memory);}finally{Marshal.FreeHGlobal(memory);}source=linked;}
    if(!ConvertStringSidToSidW("S-1-5-32-544",out admin)||!ConvertStringSidToSidW("S-1-16-8192",out medium))throw new Win32Exception();
    if(!CreateRestrictedToken(source,1,1,new[]{new SidAndAttributes {Sid=admin}},0,IntPtr.Zero,0,IntPtr.Zero,out restricted))throw new Win32Exception();
    var label=new MandatoryLabel {Label=new SidAndAttributes {Sid=medium,Attributes=0x20}};
    if(!SetTokenInformation(restricted,25,ref label,(uint)Marshal.SizeOf(typeof(MandatoryLabel))+GetLengthSid(medium)))throw new Win32Exception();
    var actual=ReadToken(restricted);if(actual.Elevated||actual.AdminEnabled||actual.IntegrityRid!=8192||actual.UserSid!=user.User.Value)throw new InvalidOperationException("restricted_token_not_unelevated_medium_owner");
    IntPtr result=restricted;restricted=IntPtr.Zero;return result;
   }} finally {if(linked!=IntPtr.Zero)CloseHandle(linked);if(restricted!=IntPtr.Zero)CloseHandle(restricted);if(admin!=IntPtr.Zero)LocalFree(admin);if(medium!=IntPtr.Zero)LocalFree(medium);}
  }
  public static Process StartMedium(string image,string arguments,string localAppData,string webViewData) {
   if(arguments!="--background"&&arguments!="--quit")throw new InvalidOperationException("restricted_launch_arguments_invalid");
   image=Path.GetFullPath(image);if(!Path.IsPathFullyQualified(image)||Path.GetFileName(image)!="CodexProxyTray.exe"||image.IndexOf('"')>=0)throw new InvalidOperationException("restricted_launch_image_invalid");
   var values=new SortedDictionary<string,string>(StringComparer.OrdinalIgnoreCase);foreach(DictionaryEntry pair in Environment.GetEnvironmentVariables())values[(string)pair.Key]=(string)pair.Value;
   values["LOCALAPPDATA"]=localAppData;values["WEBVIEW2_USER_DATA_FOLDER"]=webViewData;var block=new StringBuilder();foreach(var pair in values){if(pair.Key.IndexOf('\0')>=0||pair.Value.IndexOf('\0')>=0)throw new InvalidOperationException("restricted_environment_invalid");block.Append(pair.Key).Append('=').Append(pair.Value).Append('\0');}block.Append('\0');if(block.Length>1048576)throw new InvalidOperationException("restricted_environment_size");
   IntPtr token=IntPtr.Zero,environment=IntPtr.Zero;var created=new ProcessInfo();bool resumed=false;Process managed=null;
   try {
    token=MediumToken();environment=Marshal.StringToHGlobalUni(block.ToString());var startup=new StartupInfo {cb=(uint)Marshal.SizeOf(typeof(StartupInfo)),flags=1,show=0};
    // Suspend until the actual child token and process identity have been verified. No inherited handles or console.
    if(!CreateProcessWithTokenW(token,0,image,new StringBuilder("\""+image+"\" "+arguments),0x08000404,environment,Path.GetDirectoryName(image),ref startup,out created))throw new Win32Exception();
    NativeFileTime start,exit,kernel,user;if(!GetProcessTimes(created.process,out start,out exit,out kernel,out user))throw new Win32Exception();var path=new StringBuilder(32768);uint length=(uint)path.Capacity;if(!QueryFullProcessImageNameW(created.process,0,path,ref length)||!String.Equals(Path.GetFullPath(path.ToString()),image,StringComparison.OrdinalIgnoreCase))throw new InvalidOperationException("restricted_child_image_mismatch");
    var actual=ReadProcessToken(created.process);using(var owner=WindowsIdentity.GetCurrent()){if(actual.Elevated||actual.AdminEnabled||actual.IntegrityRid!=8192||actual.UserSid!=owner.User.Value)throw new InvalidOperationException("restricted_child_token_invalid");}
    managed=Process.GetProcessById((int)created.pid);if(managed.StartTime.ToUniversalTime().ToFileTimeUtc()!=start.Value)throw new InvalidOperationException("restricted_child_identity_changed");
    if(ResumeThread(created.thread)==0xffffffff)throw new Win32Exception();resumed=true;return managed;
   } finally {if(!resumed&&created.process!=IntPtr.Zero){TerminateProcess(created.process,2);WaitForSingleObject(created.process,5000);if(managed!=null)managed.Dispose();}if(created.thread!=IntPtr.Zero)CloseHandle(created.thread);if(created.process!=IntPtr.Zero)CloseHandle(created.process);if(environment!=IntPtr.Zero)Marshal.FreeHGlobal(environment);if(token!=IntPtr.Zero)CloseHandle(token);}
  }
  static void ValidToken(string token) {if(token==null||token.Length!=32)throw new InvalidOperationException("qa_pipe_token_invalid");foreach(char value in token)if(!Uri.IsHexDigit(value))throw new InvalidOperationException("qa_pipe_token_invalid");}
  public sealed class QaPipe : IDisposable {
   IntPtr handle;readonly int limit;readonly bool server;
   internal QaPipe(IntPtr value,int maximum,bool isServer){handle=value;limit=maximum;server=isServer;}
   public void Connect(int timeout) {var clock=Stopwatch.StartNew();while(clock.ElapsedMilliseconds<timeout){if(ConnectNamedPipe(handle,IntPtr.Zero))return;int error=Marshal.GetLastWin32Error();if(error==535)return;if(error!=536)throw new Win32Exception(error);Thread.Sleep(10);}throw new InvalidOperationException("qa_pipe_connect_timeout");}
   public uint PeerPid() {uint pid;bool ok=server?GetNamedPipeClientProcessId(handle,out pid):GetNamedPipeServerProcessId(handle,out pid);if(!ok)throw new Win32Exception();return pid;}
   public void Write(string frame) {byte[] bytes=Encoding.UTF8.GetBytes(frame);if(bytes.Length==0||bytes.Length>limit)throw new InvalidOperationException("qa_pipe_frame_size");uint written;if(!WriteFile(handle,bytes,(uint)bytes.Length,out written,IntPtr.Zero)||written!=bytes.Length)throw new InvalidOperationException("qa_pipe_write_failed");}
   public string Read(int timeout) {var clock=Stopwatch.StartNew();while(clock.ElapsedMilliseconds<timeout){uint available,remaining;if(!PeekNamedPipe(handle,IntPtr.Zero,0,IntPtr.Zero,out available,out remaining))throw new Win32Exception();if(available>0){if(remaining==0||remaining>limit)throw new InvalidOperationException("qa_pipe_frame_size");var bytes=new byte[remaining];uint read;if(!ReadFile(handle,bytes,remaining,out read,IntPtr.Zero)||read!=remaining)throw new InvalidOperationException("qa_pipe_read_failed");return new UTF8Encoding(false,true).GetString(bytes);}Thread.Sleep(10);}throw new InvalidOperationException("qa_pipe_read_timeout");}
   public void Dispose(){if(handle!=IntPtr.Zero){CloseHandle(handle);handle=IntPtr.Zero;}}
  }
  public static QaPipe TargetPipe(string token,string ownerSid) {
   ValidToken(token);using(var owner=WindowsIdentity.GetCurrent()){if(owner.User.Value!=ownerSid)throw new InvalidOperationException("qa_pipe_owner_mismatch");}
   IntPtr security=IntPtr.Zero;uint size;if(!ConvertStringSecurityDescriptorToSecurityDescriptorW("D:P(A;;GA;;;SY)(A;;GA;;;"+ownerSid+")",1,out security,out size))throw new Win32Exception();
   try {var attributes=new SecurityAttributes {length=(uint)Marshal.SizeOf(typeof(SecurityAttributes)),descriptor=security};IntPtr handle=CreateNamedPipeW("\\\\.\\pipe\\CodexProxyTray.Traffic."+token,3|0x80000,4|2|1|8,1,32768,32768,1000,ref attributes);if(handle==new IntPtr(-1))throw new Win32Exception();return new QaPipe(handle,32768,true);}finally{LocalFree(security);}
  }
  public static QaPipe BrokerPipe(uint expectedPid,string expectedImage) {
   var clock=Stopwatch.StartNew();while(clock.ElapsedMilliseconds<8000){IntPtr handle=CreateFileW("\\\\.\\pipe\\CodexProxyTray.Traffic.Service.v2",0xc0000000,0,IntPtr.Zero,3,0x110000,IntPtr.Zero);if(handle!=new IntPtr(-1)){var pipe=new QaPipe(handle,4096,false);try{uint mode=2|1;if(!SetNamedPipeHandleState(handle,ref mode,IntPtr.Zero,IntPtr.Zero))throw new Win32Exception();if(pipe.PeerPid()!=expectedPid)throw new InvalidOperationException("qa_broker_peer_mismatch");using(var process=Process.GetProcessById((int)expectedPid)){if(!String.Equals(Path.GetFullPath(process.MainModule.FileName),Path.GetFullPath(expectedImage),StringComparison.OrdinalIgnoreCase)||ProcessToken(expectedPid).UserSid!="S-1-5-18")throw new InvalidOperationException("qa_broker_image_mismatch");}return pipe;}catch{pipe.Dispose();throw;}}int error=Marshal.GetLastWin32Error();if(error!=2&&error!=231)throw new Win32Exception(error);Thread.Sleep(20);}throw new InvalidOperationException("qa_broker_connect_timeout");
  }
  public sealed class CollectorLease : IDisposable {
   IntPtr handle;public readonly uint Pid;public readonly long Created;
   public CollectorLease(ulong raw,uint expectedPid,long expectedCreated,string expectedImage) {
    handle=new IntPtr(unchecked((long)raw));try{NativeFileTime created,exit,kernel,user;if(raw==0||GetProcessId(handle)!=expectedPid||!GetProcessTimes(handle,out created,out exit,out kernel,out user)||created.Value!=expectedCreated)throw new InvalidOperationException("qa_collector_identity_mismatch");var image=new StringBuilder(32768);uint length=(uint)image.Capacity;if(!QueryFullProcessImageNameW(handle,0,image,ref length)||!String.Equals(Path.GetFullPath(image.ToString()),Path.GetFullPath(expectedImage),StringComparison.OrdinalIgnoreCase)||ReadProcessToken(handle).UserSid!="S-1-5-18")throw new InvalidOperationException("qa_collector_image_mismatch");Pid=expectedPid;Created=created.Value;}catch{Dispose();throw;}
   }
   public bool Wait(int timeout){uint result=WaitForSingleObject(handle,(uint)timeout);if(result==0)return true;if(result==258)return false;throw new Win32Exception();}
   public uint ExitCode {get{uint code;if(!GetExitCodeProcess(handle,out code))throw new Win32Exception();return code;}}
   public void Dispose(){if(handle!=IntPtr.Zero){CloseHandle(handle);handle=IntPtr.Zero;}}
  }
  public static bool OwnTraceExists(string token) {
   ValidToken(token);IntPtr memory=Marshal.AllocHGlobal(4096);try{for(int offset=0;offset<4096;offset+=4)Marshal.WriteInt32(memory,offset,0);var properties=new TraceProperties {node=new WnodeHeader {size=4096},loggerName=(uint)Marshal.SizeOf(typeof(TraceProperties))};Marshal.StructureToPtr(properties,memory,false);uint result=ControlTraceW(0,"CodexProxyTray.Network."+token,memory,0);if(result==0)return true;if(result==4201)return false;throw new Win32Exception((int)result);}finally{Marshal.FreeHGlobal(memory);}
  }
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
    $keys=@('schema','product','owner_sid','version','protocol','sha256','loader_sha256','image','service_command','phase','readiness','previous','created_parent')
    foreach($key in $keys){Require ($key -in $receipt.PSObject.Properties.Name) 'receipt_schema_missing_field'}
    Require ($receipt.schema -eq 1 -and $receipt.product -eq 'CodexProxyTray.TrafficService' -and $receipt.owner_sid -ceq $ownerSid) 'receipt_owner_or_product_invalid'
    Require ($receipt.version -ceq '1.0.0' -and $receipt.protocol -eq 2 -and $receipt.sha256 -ceq $binaryHash -and $receipt.loader_sha256 -ceq $loaderHash) 'receipt_version_or_hash_invalid'
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
function Run-ServicePipeline([string]$GuiBinary=$binary,[string]$ExpectedError,[switch]$MediumIntegrity) {
    $sandbox=Join-Path $diagnosticRoot 'pipeline-data'
    Require (-not (Test-Path -LiteralPath $sandbox)) 'pipeline_sandbox_exists'
    [void][IO.Directory]::CreateDirectory((Join-Path $sandbox 'CodexProxyTray'))
    [IO.File]::WriteAllText((Join-Path $sandbox 'CodexProxyTray/config.json'),'{"launch_target":"none","network_enabled":false,"connectivity_enabled":false,"traffic_enabled":true,"auto_check_updates":false}')
    $start={param([string]$Arguments)
        if($MediumIntegrity){return [CodexProxyTray.ServiceQaNative]::StartMedium($GuiBinary,$Arguments,$sandbox,(Join-Path $sandbox 'webview-data'))}
        $info=[Diagnostics.ProcessStartInfo]::new();$info.FileName=$GuiBinary;$info.Arguments=$Arguments;$info.WorkingDirectory=[IO.Path]::GetDirectoryName($GuiBinary);$info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.WindowStyle='Hidden'
        $info.EnvironmentVariables['LOCALAPPDATA']=$sandbox;$info.EnvironmentVariables['WEBVIEW2_USER_DATA_FOLDER']=Join-Path $sandbox 'webview-data'
        return [Diagnostics.Process]::Start($info)
    }
    $gui=$null;$pipelineFailure=$null;$final=$null
    try{
        Require (@(Get-Process -Name CodexProxyTray -ErrorAction SilentlyContinue).Count -eq 0) 'pipeline_other_gui_preserved'
        $gui=&$start '--background';$guiStarted=$gui.StartTime.ToUniversalTime();$deadline=[DateTime]::UtcNow.AddSeconds(40);$seen=[Collections.Generic.HashSet[long]]::new()
        if($MediumIntegrity){
            $actual=[CodexProxyTray.ServiceQaNative]::ProcessToken($gui.Id)
            Require (-not $actual.Elevated -and -not $actual.AdminEnabled -and $actual.IntegrityRid -eq 8192 -and $actual.UserSid -ceq $ownerSid) 'pipeline_not_unelevated_medium_owner'
            $report['medium_gui_token']=[ordered]@{elevated=$actual.Elevated;integrity_rid=$actual.IntegrityRid;administrator_enabled=$actual.AdminEnabled;owner_sid_matches=($actual.UserSid -ceq $ownerSid)}
        }
        while([DateTime]::UtcNow -lt $deadline){
            Require (-not $gui.HasExited) 'pipeline_gui_exited'
            $path=Join-Path $sandbox 'CodexProxyTray/status.json'
            if(Test-Path -LiteralPath $path){
                try{$value=Get-Content -LiteralPath $path -Raw|ConvertFrom-Json}catch{Start-Sleep -Milliseconds 100;continue}
                if($value.helper_pid -eq $gui.Id){
                    if($ExpectedError -and $value.traffic.state -eq 'unavailable' -and $value.traffic.message -like ('*'+$ExpectedError+'*')){[void]$seen.Add(-1);break}
                    if($value.traffic.state -in @('unavailable','needs_permission')){$report['pipeline_error_message']=[string]$value.traffic.message;throw 'service_pipeline_unavailable'}
                    if($value.traffic.state -eq 'collecting' -and $null -ne $value.traffic.received_bytes -and $null -ne $value.traffic.sent_bytes){[void]$seen.Add([long]$value.traffic.timestamp_ms)}
                    if($seen.Count -ge 2){break}
                }
            }
            Start-Sleep -Milliseconds 150
        }
        Require (($ExpectedError -and $seen.Contains(-1)) -or (-not $ExpectedError -and $seen.Count -ge 2)) 'service_pipeline_samples_missing'
        if(-not $ExpectedError){$report['service_pipeline_samples']=$seen.Count}
    }catch{$pipelineFailure=$_}
    finally{
        if($gui){
            try{
                if(-not $gui.HasExited){Require ($gui.StartTime.ToUniversalTime() -eq $guiStarted -and (Same-Path $gui.Path $GuiBinary)) 'pipeline_gui_identity_changed';$quitter=&$start '--quit';try{Require ($quitter.WaitForExit(10000)) 'pipeline_quit_request_timeout'}finally{$quitter.Dispose()};Require ($gui.WaitForExit(15000)) 'pipeline_gui_stop_timeout'}
                $statusPath=Join-Path $sandbox 'CodexProxyTray/status.json';if(Test-Path -LiteralPath $statusPath){$final=Get-Content -LiteralPath $statusPath -Raw|ConvertFrom-Json}
            }catch{if(-not $gui.HasExited -and $gui.StartTime.ToUniversalTime() -eq $guiStarted -and (Same-Path $gui.Path $GuiBinary)){$gui.Kill($true);[void]$gui.WaitForExit(10000)};if(-not $pipelineFailure){$pipelineFailure=$_}}
            finally{$gui.Dispose()}
        }
        $until=[DateTime]::UtcNow.AddSeconds(12)
        do{$collectors=@(Get-CimInstance Win32_Process -Filter "Name = 'CodexProxyTrayTrafficService.exe'"|Where-Object {$_.CommandLine -like '*--traffic-collector*' -and (Same-Path $_.ExecutablePath $serviceImage)});if($collectors.Count){Start-Sleep -Milliseconds 100}}while($collectors.Count -and [DateTime]::UtcNow -lt $until)
        if($collectors.Count -and -not $pipelineFailure){$pipelineFailure='pipeline_collectors_remain'}
        $report['pipeline_stop_state']=$final.traffic.state;$report['pipeline_stop_message']=[string]$final.traffic.message
        if(-not $pipelineFailure){try{No-Reparse $sandbox;Require (Is-Within (Full-Path $sandbox) $diagnosticRoot) 'pipeline_cleanup_boundary';Start-Sleep -Seconds 2;Remove-Item -LiteralPath $sandbox -Recurse -Force}catch{$pipelineFailure=$_}}
    }
    if($pipelineFailure){throw $pipelineFailure}
    Require ($final.helper_pid -gt 0 -and -not $final.backend_ready -and $final.traffic.state -eq 'disabled') 'pipeline_stop_ack_not_confirmed'
    $report['service_pipeline_stop_ack']=$true;$report['service_pipeline_collectors_exited']=$true
}
function Run-TargetScopeRejection([switch]$AfterEmptyTargets) {
    $owned=Assert-OwnedInstall
    if($owned.service.State -eq 1){Start-Service -Name $serviceName -ErrorAction Stop -WarningAction Stop;$controller=Get-Service -Name $serviceName;try{$controller.WaitForStatus([ServiceProcess.ServiceControllerStatus]::Running,[TimeSpan]::FromSeconds($ServiceTimeoutSeconds))}finally{$controller.Dispose()}}
    $owned=Assert-OwnedInstall;Require ($owned.service.State -eq 4 -and $owned.service.ProcessId -gt 0) 'scope_broker_not_running'
    $token=[Guid]::NewGuid().ToString('N');$targetPipe=$null;$brokerPipe=$null;$collector=$null;$scopeFailure=$null;$acknowledged=$false;$exitCode=$null
    try {
        Require (-not [CodexProxyTray.ServiceQaNative]::OwnTraceExists($token)) 'scope_trace_collision'
        $targetPipe=[CodexProxyTray.ServiceQaNative]::TargetPipe($token,$ownerSid)
        $brokerPipe=[CodexProxyTray.ServiceQaNative]::BrokerPipe($owned.service.ProcessId,$serviceImage)
        $parent=[Diagnostics.Process]::GetCurrentProcess();try{$created=$parent.StartTime.ToUniversalTime().ToFileTimeUtc()}finally{$parent.Dispose()}
        # A same-SID PowerShell client is intentionally allowed to request only our fixed collector.
        # It must still be unable to widen Targets to the live PowerShell process itself.
        $brokerPipe.Write((@{protocol=2;token=$token;parent=@{pid=$PID;created_filetime=$created}}|ConvertTo-Json -Compress -Depth 4))
        $reply=$brokerPipe.Read(8000)|ConvertFrom-Json
        Require ($reply.protocol -eq 2 -and [uint64]$reply.handle -gt 0 -and $null -eq $reply.error -and $reply.child.pid -gt 0 -and $reply.child.created_filetime -gt 0) 'scope_broker_reply_invalid'
        $collector=[CodexProxyTray.ServiceQaNative+CollectorLease]::new([uint64]$reply.handle,[uint32]$reply.child.pid,[long]$reply.child.created_filetime,$serviceImage)
        $brokerPipe.Write('{"protocol":2}');$brokerPipe.Dispose();$brokerPipe=$null
        $targetPipe.Connect(8000);Require ($targetPipe.PeerPid() -eq $collector.Pid) 'scope_collector_pipe_peer_mismatch'
        if($AfterEmptyTargets){
            $targetPipe.Write('{"type":"targets","protocol":2,"targets":[]}');$sample=$targetPipe.Read(8000)|ConvertFrom-Json
            Require ($sample.protocol -eq 2 -and -not $sample.stopped -and $sample.snapshot.state -eq 'collecting' -and $sample.snapshot.target_count -eq 0) 'scope_empty_targets_not_sampling'
            Require ([CodexProxyTray.ServiceQaNative]::OwnTraceExists($token)) 'scope_owned_trace_not_started'
        }
        $targetPipe.Write((@{type='targets';protocol=2;targets=@(@{pid=$PID;created_filetime=$created})}|ConvertTo-Json -Compress -Depth 5))
        $deadline=[DateTime]::UtcNow.AddSeconds(8);$rejected=$null
        do {$frame=$targetPipe.Read(4000)|ConvertFrom-Json;Require ($frame.protocol -eq 2) 'scope_collector_protocol_mismatch';if($frame.stopped){$rejected=$frame;break}}while([DateTime]::UtcNow -lt $deadline)
        Require ($null -ne $rejected -and $rejected.snapshot.state -eq 'unavailable' -and $rejected.snapshot.message -like '*不属于已核验*') 'alive_out_of_scope_target_not_rejected'
        $targetPipe.Write('{"type":"stopped_received","protocol":2}');$acknowledged=$true
        Require ($collector.Wait(10000)) 'scope_collector_exit_not_confirmed';$exitCode=$collector.ExitCode
        Require ($exitCode -eq 2) 'scope_collector_rejection_exit_invalid'
        Require (-not [CodexProxyTray.ServiceQaNative]::OwnTraceExists($token)) 'scope_owned_trace_remains'
        $key=if($AfterEmptyTargets){'subsequent_alive_target_rejection'}else{'initial_alive_target_rejection'}
        $report[$key]=[ordered]@{status='passed';client='same_owner_powershell';collector_identity_verified=$true;stopped_error_frame=$true;stop_acknowledged=$true;exit_code=$exitCode;owned_trace_absent=$true}
    } catch {$scopeFailure=$_}
    finally {
        if($collector -and -not $collector.Wait(0) -and $targetPipe){
            # If a regression accepted the target, stop only this fixed, identity-checked collector.
            try {if(-not $acknowledged){$targetPipe.Write('{"type":"stop","protocol":2}');$deadline=[DateTime]::UtcNow.AddSeconds(5);do{$frame=$targetPipe.Read(2000)|ConvertFrom-Json;if($frame.protocol -eq 2 -and $frame.stopped){$targetPipe.Write('{"type":"stopped_received","protocol":2}');break}}while([DateTime]::UtcNow -lt $deadline)}}catch{}
        }
        if($brokerPipe){$brokerPipe.Dispose()};if($targetPipe){$targetPipe.Dispose()}
        if($collector){try{if(-not $collector.Wait(8000)){if(-not $scopeFailure){$scopeFailure='scope_collector_cleanup_unconfirmed'};$report['scope_collector_cleanup']='blocked'}}finally{$collector.Dispose()}}
    }
    if($scopeFailure){throw $scopeFailure}
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
    Run-Case 'unelevated_medium_owner_gui_collects_and_stops' {
        Run-ServicePipeline -MediumIntegrity
        Require ((Assert-OwnedInstall).receipt_hash -ceq $firstReceipt) 'medium_gui_changed_service_install'
        $report['unelevated_medium_service_roundtrip_and_stop_verified']=$true
    }
    Run-Case 'same_owner_non_gui_client_cannot_start_out_of_scope_targets' {Run-TargetScopeRejection}
    Run-Case 'same_owner_non_gui_client_cannot_add_out_of_scope_targets' {Run-TargetScopeRejection -AfterEmptyTargets}
    Run-Case 'different_gui_hash_reuses_installed_compatible_service' {
        $variant=Join-Path $diagnosticRoot 'CodexProxyTray.exe';$variantLoader=Join-Path $diagnosticRoot $loaderName
        Copy-Item -LiteralPath $binary -Destination $variant;Copy-Item -LiteralPath $sourceLoader -Destination $variantLoader
        [void]$diagnosticFiles.Add($variant);[void]$diagnosticFiles.Add($variantLoader)
        # A PE overlay changes only this isolated GUI copy, proving IPC has no whole-file hash coupling.
        $stream=[IO.File]::Open($variant,[IO.FileMode]::Append,[IO.FileAccess]::Write);try{$marker=[Text.Encoding]::UTF8.GetBytes('compatible-gui-hash-fixture-v1');$stream.Write($marker,0,$marker.Length)}finally{$stream.Dispose()}
        $variantHash=(Get-FileHash -LiteralPath $variant -Algorithm SHA256).Hash.ToLowerInvariant();Require ($variantHash -cne $binaryHash) 'gui_hash_fixture_not_distinct'
        Run-ServicePipeline -GuiBinary $variant
        Require ((Assert-OwnedInstall).receipt_hash -ceq $firstReceipt) 'compatible_gui_changed_service_install'
        $report['different_gui_sha256']=$variantHash;$report['compatible_hash_independence_verified']=$true
    }
    Run-Case 'unsupported_service_protocol_and_integrity_are_rejected' {
        $receiptPath=Join-Path $serviceDirectory 'receipt.json';$original=[IO.File]::ReadAllBytes($receiptPath)
        try {
            $receipt=[Text.Encoding]::UTF8.GetString($original)|ConvertFrom-Json;$receipt.protocol=99
            [IO.File]::WriteAllText($receiptPath,($receipt|ConvertTo-Json -Depth 5));Run-ServicePipeline -ExpectedError '协议'
            [IO.File]::WriteAllBytes($receiptPath,$original);$receipt=[Text.Encoding]::UTF8.GetString($original)|ConvertFrom-Json;$receipt.sha256='0'*64
            [IO.File]::WriteAllText($receiptPath,($receipt|ConvertTo-Json -Depth 5));Run-ServicePipeline -ExpectedError '校验不匹配'
        } finally {[IO.File]::WriteAllBytes($receiptPath,$original)}
        $null=Assert-OwnedInstall;$report['protocol_and_integrity_rejections_verified']=$true
    }
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
    try {$report.diagnostic_cleanup=Remove-Diagnostics}catch{$report.diagnostic_cleanup='blocked';$report.diagnostic_cleanup_code=Failure-Code $_;$failure=$true;$report.status='failed'}
    if($failure){try{$state=Inspect-Service;if($state){$report.retained_service_state=$state.State;$report.retained_service_win32_exit=$state.Win32Exit;$report.retained_service_specific_exit=$state.SpecificExit}}catch{}}
    if($serviceProcess){$serviceProcess.Dispose()}
    $report.tests=[ordered]@{pass=@($cases|Where-Object status -eq 'passed').Count;fail=@($cases|Where-Object status -eq 'failed').Count}
    $report.finished_at=[DateTime]::UtcNow.ToString('o')
    [void](Check-EvidencePath $evidenceRoot -AllowExisting)
    Write-NewJson (Join-Path $evidenceRoot 'service-checks.json') $report
}
if($failure){throw 'Traffic service QA failed; inspect sanitized service-checks.json. Unknown or uncertain objects were preserved.'}
Write-Output ('Traffic service QA passed: '+$cases.Count+' cases; owned service and directory removed.')
