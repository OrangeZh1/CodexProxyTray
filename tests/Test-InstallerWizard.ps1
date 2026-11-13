param([Parameter(Mandatory=$true)][string]$ReleaseDirectory,[Parameter(Mandatory=$true)][string]$EvidenceDirectory)
$ErrorActionPreference='Stop'
if(-not $IsWindows -or -not [Environment]::Is64BitProcess -or $env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted'){throw 'Run wizard QA only on an isolated GitHub-hosted Windows runner.'}
$release=(Resolve-Path -LiteralPath $ReleaseDirectory).Path
$manifest=Get-Content -LiteralPath (Join-Path $release 'CodexProxyTray-update.json') -Raw -Encoding utf8|ConvertFrom-Json
$setup=(Resolve-Path -LiteralPath (Join-Path $release ("CodexProxyTray-v$($manifest.version)-windows-x64-setup.exe"))).Path
$evidence=[IO.Path]::GetFullPath($EvidenceDirectory);$boundary=[IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\')+'\'
if(-not $evidence.StartsWith($boundary,[StringComparison]::OrdinalIgnoreCase) -or (Test-Path -LiteralPath $evidence)){throw 'Use a fresh evidence directory under RUNNER_TEMP.'}
foreach($path in @($setup,$release,[IO.Path]::GetDirectoryName($evidence))){$cursor=$path;while($cursor){if((Test-Path -LiteralPath $cursor) -and ((Get-Item -LiteralPath $cursor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)){throw 'Wizard QA paths must not contain reparse points.'};$parent=[IO.Path]::GetDirectoryName($cursor);if($parent -eq $cursor){break};$cursor=$parent}}
$defaultRoot=Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Programs\CodexProxyTray'
$reg='HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexProxyTray'
$shortcut=Join-Path ([Environment]::GetFolderPath('Programs')) 'CodexProxyTray.lnk'
$desktop=Join-Path ([Environment]::GetFolderPath('DesktopDirectory')) 'CodexProxyTray.lnk'
foreach($path in @($defaultRoot,$reg,$shortcut,$desktop)){if(Test-Path -LiteralPath $path){throw 'A preexisting installation or product entry was preserved; wizard QA requires a fresh runner.'}}
if(@(Get-Process -Name CodexProxyTray -ErrorAction SilentlyContinue).Count){throw 'A preexisting helper was preserved.'}
[void][IO.Directory]::CreateDirectory($evidence)
Add-Type -AssemblyName System.Drawing
if(-not ('CodexProxyTray.WizardSession' -as [type])){Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
namespace CodexProxyTray {
 public sealed class WizardControl {public long Handle;public int Id;public string Class,Text;public bool Visible,Enabled;public int Checked;public int X,Y,Width,Height;}
 public sealed class WizardState {public uint Pid;public long Created,Window;public string Title;public bool Visible;public int X,Y,Width,Height;public WizardControl[] Controls;}
 public sealed class WizardSession : IDisposable {
  delegate bool EnumProc(IntPtr window,IntPtr value);
  [StructLayout(LayoutKind.Sequential)] struct Rect {public int left,top,right,bottom;}
  [StructLayout(LayoutKind.Sequential)] struct Time {public uint low,high;public long Value {get{return ((long)high<<32)|low;}}}
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool GetProcessTimes(IntPtr process,out Time created,out Time exited,out Time kernel,out Time user);
  [DllImport("kernel32.dll",SetLastError=true)] static extern uint GetExitCodeProcess(IntPtr process,out uint code);
  [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool QueryFullProcessImageNameW(IntPtr process,uint flags,StringBuilder path,ref uint length);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback,IntPtr value);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr window,EnumProc callback,IntPtr value);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window,out uint pid);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr window);
  [DllImport("user32.dll")] static extern bool IsWindowEnabled(IntPtr window);
  [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr window);
  [DllImport("user32.dll")] static extern IntPtr GetDlgItem(IntPtr window,int id);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr window,StringBuilder value,int length);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr window,out Rect rect);
  [DllImport("user32.dll",SetLastError=true)] static extern bool PrintWindow(IntPtr window,IntPtr dc,uint flags);
  [DllImport("user32.dll")] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
  [DllImport("user32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern IntPtr SendMessageTimeoutW(IntPtr window,uint message,IntPtr wparam,IntPtr lparam,uint flags,uint timeout,out UIntPtr result);
  [DllImport("user32.dll",EntryPoint="SendMessageTimeoutW",CharSet=CharSet.Unicode,SetLastError=true)] static extern IntPtr SendText(IntPtr window,uint message,IntPtr wparam,string text,uint flags,uint timeout,out UIntPtr result);
  [DllImport("user32.dll",EntryPoint="SendMessageTimeoutW",CharSet=CharSet.Unicode,SetLastError=true)] static extern IntPtr ReadText(IntPtr window,uint message,IntPtr wparam,StringBuilder text,uint flags,uint timeout,out UIntPtr result);
  readonly Process process;readonly IntPtr native;readonly uint pid;readonly long created;readonly string image;
  public WizardSession(Process owned,string expectedImage){process=owned;native=owned.Handle;pid=(uint)owned.Id;image=Path.GetFullPath(expectedImage);Time at,exit,kernel,user;if(!GetProcessTimes(native,out at,out exit,out kernel,out user))throw new Win32Exception();created=at.Value;EnsureProcess();}
  void EnsureProcess(){uint code;if(GetExitCodeProcess(native,out code)==0||code!=259)throw new InvalidOperationException("wizard_process_not_running");Time at,exit,kernel,user;if(!GetProcessTimes(native,out at,out exit,out kernel,out user)||at.Value!=created)throw new InvalidOperationException("wizard_process_identity_changed");var path=new StringBuilder(32768);uint length=(uint)path.Capacity;if(!QueryFullProcessImageNameW(native,0,path,ref length)||!String.Equals(Path.GetFullPath(path.ToString()),image,StringComparison.OrdinalIgnoreCase))throw new InvalidOperationException("wizard_process_image_changed");}
  void EnsureWindow(IntPtr window){EnsureProcess();uint actual;if(window==IntPtr.Zero||GetWindowThreadProcessId(window,out actual)==0||actual!=pid)throw new InvalidOperationException("wizard_window_not_owned");}
  static string Text(IntPtr window){var text=new StringBuilder(4096);UIntPtr result;if(ReadText(window,0x0d,new IntPtr(text.Capacity),text,2,2000,out result)==IntPtr.Zero)return String.Empty;return text.ToString();}
  static string Class(IntPtr window){var text=new StringBuilder(256);GetClassNameW(window,text,text.Capacity);return text.ToString();}
  public long MainWindow(){EnsureProcess();var candidates=new List<IntPtr>();EnumWindows((window,value)=>{uint actual;GetWindowThreadProcessId(window,out actual);if(actual==pid&&IsWindowVisible(window)&&Class(window)=="#32770"&&Text(window).Contains("CodexProxyTray"))candidates.Add(window);return true;},IntPtr.Zero);if(candidates.Count!=1)return 0;return candidates[0].ToInt64();}
  public WizardState Snapshot(long raw){var window=new IntPtr(raw);EnsureWindow(window);IntPtr old=SetThreadDpiAwarenessContext(new IntPtr(-4));try{Rect area;if(!GetWindowRect(window,out area))throw new Win32Exception();var children=new List<WizardControl>();EnumChildWindows(window,(child,value)=>{uint actual;GetWindowThreadProcessId(child,out actual);if(actual!=pid)return true;Rect rect;GetWindowRect(child,out rect);UIntPtr state;int selected=SendMessageTimeoutW(child,0xf0,IntPtr.Zero,IntPtr.Zero,2,1000,out state)==IntPtr.Zero?-1:unchecked((int)state.ToUInt64());children.Add(new WizardControl{Handle=child.ToInt64(),Id=GetDlgCtrlID(child),Class=Class(child),Text=Text(child),Visible=IsWindowVisible(child),Enabled=IsWindowEnabled(child),Checked=selected,X=rect.left,Y=rect.top,Width=rect.right-rect.left,Height=rect.bottom-rect.top});return true;},IntPtr.Zero);return new WizardState{Pid=pid,Created=created,Window=raw,Title=Text(window),Visible=IsWindowVisible(window),X=area.left,Y=area.top,Width=area.right-area.left,Height=area.bottom-area.top,Controls=children.ToArray()};}finally{if(old!=IntPtr.Zero)SetThreadDpiAwarenessContext(old);}}
  public long Button(long root,int id){IntPtr window=new IntPtr(root);EnsureWindow(window);IntPtr button=GetDlgItem(window,id);EnsureWindow(button);return button.ToInt64();}
  public void Click(long raw){IntPtr button=new IntPtr(raw);EnsureWindow(button);string text=Text(button);if(Class(button)!="Button"||!IsWindowVisible(button)||!IsWindowEnabled(button)||text.IndexOf("安装",StringComparison.Ordinal)>=0||text.IndexOf("Install",StringComparison.OrdinalIgnoreCase)>=0)throw new InvalidOperationException("wizard_install_or_invalid_button_refused");UIntPtr result;if(SendMessageTimeoutW(button,0xf5,IntPtr.Zero,IntPtr.Zero,2,3000,out result)==IntPtr.Zero)throw new InvalidOperationException("wizard_click_not_acknowledged");}
  public void SetDirectory(long raw,string path){IntPtr edit=new IntPtr(raw);EnsureWindow(edit);if(Class(edit)!="Edit"||GetDlgCtrlID(edit)!=1019||!IsWindowVisible(edit))throw new InvalidOperationException("wizard_directory_control_invalid");UIntPtr result;if(SendText(edit,0x0c,IntPtr.Zero,path,2,3000,out result)==IntPtr.Zero)throw new InvalidOperationException("wizard_directory_not_set");}
  public bool Capture(long raw,IntPtr dc){IntPtr window=new IntPtr(raw);EnsureWindow(window);IntPtr old=SetThreadDpiAwarenessContext(new IntPtr(-4));try{return IsWindowVisible(window)&&PrintWindow(window,dc,2);}finally{if(old!=IntPtr.Zero)SetThreadDpiAwarenessContext(old);}}
  public long Confirmation(){EnsureProcess();long found=0;EnumWindows((window,value)=>{uint actual;GetWindowThreadProcessId(window,out actual);if(actual==pid&&IsWindowVisible(window)&&Class(window)=="#32770"&&GetDlgItem(window,6)!=IntPtr.Zero){if(found!=0)throw new InvalidOperationException("wizard_multiple_confirmations");found=window.ToInt64();}return true;},IntPtr.Zero);return found;}
  public void CloseOwned(long root){IntPtr window=new IntPtr(root);EnsureWindow(window);UIntPtr result;SendMessageTimeoutW(window,0x10,IntPtr.Zero,IntPtr.Zero,2,2000,out result);}
  public void KillOwned(){EnsureProcess();process.Kill(false);}
  public void Dispose(){GC.KeepAlive(process);}
 }
}
'@}
$process=$null;$session=$null;$window=0L;$failure=$null;$screens=[Collections.Generic.List[object]]::new();$checks=[Collections.Generic.List[string]]::new()
$report=[ordered]@{schema=1;status='running';scope='Only this fresh NSIS wizard: welcome, directory, desktop choice, back and cancel. Install is never clicked; no business GUI, services or registry writes are requested.';setup_sha256=(Get-FileHash -LiteralPath $setup -Algorithm SHA256).Hash.ToLowerInvariant();version=$manifest.version;checks=$checks;pages=$screens;cleanup='pending'}
function Require([bool]$Condition,[string]$Code){if(-not $Condition){throw $Code};$checks.Add($Code)}
function Observe([scriptblock]$Condition,[string]$Code){$deadline=[DateTime]::UtcNow.AddSeconds(15);do{if($process.HasExited){throw 'wizard_exited_before_cancel'};$candidate=$session.MainWindow();if($candidate){$script:window=$candidate;$state=$session.Snapshot($window);if(& $Condition $state){return $state}};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $deadline);throw $Code}
function Capture([string]$Name,$State){
    Require ($State.Visible -and $State.Width -ge 300 -and $State.Height -ge 200 -and $State.Width -le 4096 -and $State.Height -le 4096) 'owned_wizard_geometry_visible'
    $bitmap=[Drawing.Bitmap]::new($State.Width,$State.Height,[Drawing.Imaging.PixelFormat]::Format24bppRgb);$graphics=[Drawing.Graphics]::FromImage($bitmap);$ok=$false
    try{$graphics.Clear([Drawing.Color]::Magenta);$dc=$graphics.GetHdc();try{$ok=$session.Capture($State.Window,$dc)}finally{$graphics.ReleaseHdc($dc)};$png=Join-Path $evidence ($Name+'.png');$bitmap.Save($png,[Drawing.Imaging.ImageFormat]::Png);$colors=[Collections.Generic.HashSet[int]]::new();for($y=0;$y -lt $bitmap.Height;$y+=[Math]::Max(1,[int]($bitmap.Height/24))){for($x=0;$x -lt $bitmap.Width;$x+=[Math]::Max(1,[int]($bitmap.Width/32))){[void]$colors.Add($bitmap.GetPixel($x,$y).ToArgb())}};$record=[ordered]@{name=$Name;pid=$State.Pid;created_filetime=$State.Created;window=$State.Window;capture_api='PrintWindow';capture_succeeded=$ok;sampled_colors=$colors.Count;image=($Name+'.png');controls=$State.Controls};$screens.Add($record);[IO.File]::WriteAllText((Join-Path $evidence ($Name+'.json')),($record|ConvertTo-Json -Depth 6),[Text.UTF8Encoding]::new($false));Require ($ok -and $colors.Count -ge 8) 'wizard_screenshot_is_not_blank_or_unavailable'}finally{$graphics.Dispose();$bitmap.Dispose()}
}
function Next($State){$button=@($State.Controls|Where-Object {$_.Id -eq 1 -and $_.Class -eq 'Button' -and $_.Visible -and $_.Enabled})[0];Require ($null -ne $button -and $button.Text.Contains('下一步')) 'next_button_is_not_install';$session.Click($button.Handle)}
function Back {$session.Click($session.Button($window,3))}
try {
    $info=[Diagnostics.ProcessStartInfo]::new($setup);$info.UseShellExecute=$false;$info.WindowStyle='Normal';$info.WorkingDirectory=$release;$process=[Diagnostics.Process]::Start($info);$session=[CodexProxyTray.WizardSession]::new($process,$setup)
    $welcome=Observe {param($s) @($s.Controls|Where-Object {$_.Visible -and $_.Text -ceq '此向导将引导你安装 CodexProxyTray。点击“下一步”选择安装位置和快捷方式。'}).Count -eq 1} 'welcome_chinese_original_not_found'
    Capture '01-welcome' $welcome;Next $welcome
    $directory=Observe {param($s) @($s.Controls|Where-Object {$_.Visible -and $_.Class -eq 'Edit' -and $_.Id -eq 1019}).Count -eq 1} 'directory_page_not_found'
    $edit=@($directory.Controls|Where-Object {$_.Visible -and $_.Class -eq 'Edit' -and $_.Id -eq 1019})[0];Require ($edit.Text -ieq $defaultRoot) 'default_is_current_user_programs';Capture '02-default-directory' $directory
    $custom=Join-Path $env:RUNNER_TEMP ('自选 中文 空格\CodexProxyTray-'+[Guid]::NewGuid().ToString('N'));$session.SetDirectory($edit.Handle,$custom)
    $directory=Observe {param($s) @($s.Controls|Where-Object {$_.Visible -and $_.Id -eq 1019 -and $_.Text -ceq $custom}).Count -eq 1} 'custom_unicode_directory_not_retained';Capture '03-custom-directory' $directory;Next $directory
    $icons=Observe {param($s) @($s.Controls|Where-Object {$_.Visible -and $_.Class -eq 'Button' -and $_.Text -ceq '创建桌面快捷方式'}).Count -eq 1} 'icon_options_not_found'
    $checkbox=@($icons.Controls|Where-Object {$_.Visible -and $_.Class -eq 'Button' -and $_.Text -ceq '创建桌面快捷方式'})[0];Require ($checkbox.Checked -eq 1) 'desktop_default_checked_on_fresh_install';$visibleText=@($icons.Controls|Where-Object Visible|ForEach-Object Text) -join "`n";Require ($visibleText.Contains($custom)) 'icon_page_shows_selected_directory';Require ($visibleText.Contains('开始菜单快捷方式将始终创建。')) 'start_menu_is_always_created_copy';Capture '04-icon-options' $icons
    $session.Click($checkbox.Handle);$icons=Observe {param($s) @($s.Controls|Where-Object {$_.Visible -and $_.Text -ceq '创建桌面快捷方式' -and $_.Checked -eq 0}).Count -eq 1} 'desktop_checkbox_did_not_toggle';Back
    $returned=Observe {param($s) @($s.Controls|Where-Object {$_.Visible -and $_.Id -eq 1019 -and $_.Text -ceq $custom}).Count -eq 1} 'back_lost_custom_directory';Capture '05-returned-directory' $returned;Next $returned
    $icons=Observe {param($s) @($s.Controls|Where-Object {$_.Visible -and $_.Text -ceq '创建桌面快捷方式' -and $_.Checked -eq 0}).Count -eq 1} 'back_lost_desktop_choice';Capture '06-returned-options' $icons
    $session.Click($session.Button($window,2));$deadline=[DateTime]::UtcNow.AddSeconds(10)
    while(-not $process.HasExited -and [DateTime]::UtcNow -lt $deadline){$confirm=$session.Confirmation();if($confirm){$session.Click($session.Button($confirm,6));break};Start-Sleep -Milliseconds 100}
    Require ($process.WaitForExit(10000) -and $process.ExitCode -eq 1) 'wizard_cancelled_without_install'
    foreach($path in @($defaultRoot,$custom,$reg,$shortcut,$desktop)){Require (-not(Test-Path -LiteralPath $path)) 'wizard_created_no_code_registration_or_shortcut'}
    Require (@(Get-Process -Name CodexProxyTray -ErrorAction SilentlyContinue).Count -eq 0) 'wizard_started_no_business_gui';$report.status='passed';$report.cleanup='owned_wizard_exited_after_cancel'
}catch{$failure=$_;$report.status='failed';$report.error=$_.Exception.Message}
finally{
    if($process){if(-not $process.HasExited){try{if($session -and $window){$session.CloseOwned($window);$confirm=$session.Confirmation();if($confirm){$session.Click($session.Button($confirm,6))}};if(-not $process.WaitForExit(5000)){$session.KillOwned();[void]$process.WaitForExit(5000);$report.cleanup='owned_fixture_required_forced_cleanup';$report.status='failed'}}catch{$report.cleanup='owned_fixture_cleanup_unconfirmed';$report.status='failed';if(-not $failure){$failure=$_}}};if($session){$session.Dispose()};$process.Dispose()}
    $report.finished_at=[DateTime]::UtcNow.ToString('o');[IO.File]::WriteAllText((Join-Path $evidence 'wizard-checks.json'),($report|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
}
if($failure -or $report.status -ne 'passed'){throw 'Installer wizard QA failed; inspect wizard-checks.json and captured pages.'}
Write-Output 'Native NSIS Chinese pages, directory, icon choice, back/cancel and nonblank screenshots passed without installing.'
