Target x86-unicode
Unicode true
RequestExecutionLevel user
SetCompressor /SOLID lzma
Name "CodexProxyTray"
OutFile "${OUTPUT_FILE}"
InstallDir "$LOCALAPPDATA\Programs\CodexProxyTray"
InstallDirRegKey HKCU64 "Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexProxyTray" "InstallLocation"
ShowInstDetails show
ShowUninstDetails show
VIProductVersion "${APP_VERSION}.0"
VIAddVersionKey /LANG=2052 "ProductName" "CodexProxyTray"
VIAddVersionKey /LANG=2052 "ProductVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=2052 "FileVersion" "${APP_VERSION}.0"
VIAddVersionKey /LANG=2052 "FileDescription" "CodexProxyTray 当前用户安装程序"
VIAddVersionKey /LANG=2052 "CompanyName" "OrangeZh1"
VIAddVersionKey /LANG=2052 "LegalCopyright" "Copyright (c) 2026 OrangeZh1"

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"
!include "WinVer.nsh"
!include "FileFunc.nsh"
!include "nsDialogs.nsh"
!define MUI_ICON "${ICON_PATH}"
!define MUI_UNICON "${ICON_PATH}"
!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TEXT "此向导将引导你安装 CodexProxyTray。点击“下一步”选择安装位置和快捷方式。"
!define MUI_FINISHPAGE_RUN "$INSTDIR\current\CodexProxyTray.exe"
!define MUI_FINISHPAGE_RUN_TEXT "运行 CodexProxyTray"
!define MUI_UNCONFIRMPAGE_TEXT_TOP "将移除当前用户的 CodexProxyTray、开始菜单和软件列表入口。配置、历史及日志会保留。可选流量辅助服务独立管理。"
!insertmacro MUI_PAGE_WELCOME
!define MUI_PAGE_CUSTOMFUNCTION_SHOW DirectoryShow
!define MUI_PAGE_CUSTOMFUNCTION_LEAVE DirectoryLeave
!insertmacro MUI_PAGE_DIRECTORY
Page custom IconOptionsShow IconOptionsLeave
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"

Var BackendExit
Var BackendCommand
Var BackendOutput
Var SystemPowerShell
Var BootstrapText
Var BootstrapCount
Var BootstrapBom
Var RegisteredRoot
Var DesktopChoice
Var DesktopCheckbox
Var OptionsDialog

!macro DefineBootstrapDiagnostic Prefix
Function ${Prefix}BootstrapDiagnostic
  Push $8
  Push $9
  StrCpy $BootstrapText $BootstrapText 400
  DetailPrint $BootstrapText
  ${If} $BootstrapCount < 10
    System::Call 'kernel32::GetStdHandle(i -11) p.r9'
    System::Call 'kernel32::GetFileType(p r9) i.r8'
    ${If} $8 == 1
    ${OrIf} $8 == 3
      ; Borrow the caller's redirected stdout. Do not close it or open a console.
      ; /BOM uses seeking; emit the first UTF16 BOM directly so pipes work too.
      ${If} $BootstrapBom != 1
        FileWriteWord $9 0xFEFF
        StrCpy $BootstrapBom 1
      ${EndIf}
      FileWriteUTF16LE $9 "$BootstrapText$\r$\n"
      IntOp $BootstrapCount $BootstrapCount + 1
    ${EndIf}
  ${EndIf}
  Pop $9
  Pop $8
FunctionEnd
!macroend
!insertmacro DefineBootstrapDiagnostic ""
!insertmacro DefineBootstrapDiagnostic "un."

!macro CaptureEnvironment Original Present Value Prefix
  System::Call 'kernel32::GetEnvironmentVariableW(w "${Original}", p 0, i 0) i.r0 ?e'
  Pop $1
  ${If} $0 == 0
    ${If} $1 == 203
      System::Call 'kernel32::SetEnvironmentVariableW(w "${Present}", w "0") i.r3 ?e'
      Pop $1
      System::Call 'kernel32::SetEnvironmentVariableW(w "${Value}", p 0)'
    ${ElseIf} $1 == 0
      System::Call 'kernel32::SetEnvironmentVariableW(w "${Present}", w "1") i.r3 ?e'
      Pop $1
      System::Call 'kernel32::SetEnvironmentVariableW(w "${Value}", w "")'
    ${Else}
      StrCpy $BootstrapText "stage=capture;variable=${Original};operation=query;return=$0;winerror=$1"
      Call ${Prefix}BootstrapDiagnostic
      SetErrorLevel 2
      Abort
    ${EndIf}
  ${Else}
    ${If} $0 > 32768
      StrCpy $BootstrapText "stage=capture;variable=${Original};operation=size;length=$0"
      Call ${Prefix}BootstrapDiagnostic
      SetErrorLevel 2
      Abort
    ${EndIf}
    IntOp $1 $0 * 2
    System::Alloc $1
    Pop $2
    ${If} $2 == 0
      StrCpy $BootstrapText "stage=capture;variable=${Original};operation=allocate;failed=1"
      Call ${Prefix}BootstrapDiagnostic
      SetErrorLevel 2
      Abort
    ${EndIf}
    System::Call 'kernel32::GetEnvironmentVariableW(w "${Original}", p r2, i r0) i.r3 ?e'
    Pop $1
    ${If} $3 >= $0
      System::Free $2
      StrCpy $BootstrapText "stage=capture;variable=${Original};operation=read;return=$3;capacity=$0;winerror=$1"
      Call ${Prefix}BootstrapDiagnostic
      SetErrorLevel 2
      Abort
    ${EndIf}
    ${If} $3 == 0
    ${AndIf} $1 != 0
      System::Free $2
      StrCpy $BootstrapText "stage=capture;variable=${Original};operation=read;return=$3;winerror=$1"
      Call ${Prefix}BootstrapDiagnostic
      SetErrorLevel 2
      Abort
    ${EndIf}
    ; A positive return is success; its LastError may be an unrelated stale value.
    System::Call 'kernel32::SetEnvironmentVariableW(w "${Value}", p r2) i.r3 ?e'
    Pop $1
    System::Free $2
    ${If} $3 == 0
      StrCpy $BootstrapText "stage=capture;variable=${Original};operation=copy;return=$3;winerror=$1"
      Call ${Prefix}BootstrapDiagnostic
      SetErrorLevel 2
      Abort
    ${EndIf}
    System::Call 'kernel32::SetEnvironmentVariableW(w "${Present}", w "1") i.r3 ?e'
    Pop $1
  ${EndIf}
  ${If} $3 == 0
    StrCpy $BootstrapText "stage=capture;variable=${Original};operation=presence;return=$3;winerror=$1"
    Call ${Prefix}BootstrapDiagnostic
    SetErrorLevel 2
    Abort
  ${EndIf}
!macroend

!macro CaptureOriginalEnvironment Prefix
  StrCpy $BootstrapText "stage=environment-capture-start"
  Call ${Prefix}BootstrapDiagnostic
  !insertmacro CaptureEnvironment PSModulePath CODEX_PROXY_UPDATER_MODULES_PRESENT CODEX_PROXY_UPDATER_MODULES "${Prefix}"
  !insertmacro CaptureEnvironment PSExecutionPolicyPreference CODEX_PROXY_UPDATER_POLICY_PRESENT CODEX_PROXY_UPDATER_POLICY "${Prefix}"
  StrCpy $BootstrapText "stage=environment-capture-complete"
  Call ${Prefix}BootstrapDiagnostic
!macroend

!macro EmitBackendResult Prefix
  StrCpy $BootstrapText "stage=backend-return;return=$BackendExit"
  Call ${Prefix}BootstrapDiagnostic
  ${If} $BackendExit != 0
    StrCpy $BootstrapText "backend-output-1=$BackendOutput" 400
    Call ${Prefix}BootstrapDiagnostic
    StrCpy $BootstrapText $BackendOutput 400 382
    Call ${Prefix}BootstrapDiagnostic
    StrCpy $BootstrapText $BackendOutput 400 782
    Call ${Prefix}BootstrapDiagnostic
  ${EndIf}
!macroend

!macro EmitLauncherCheck Prefix
  System::Call 'kernel32::IsWow64Process(p -1, *i.r0) i.r1'
  StrLen $2 $BackendCommand
  StrCpy $3 0
  IfFileExists "$SystemPowerShell" 0 +2
    StrCpy $3 1
  StrCpy $BootstrapText "stage=launcher-check;wow64=$0;query=$1;file_exists=$3;command_chars=$2"
  Call ${Prefix}BootstrapDiagnostic
!macroend

!macro ClearPrivateEnvironment
  System::Call 'kernel32::SetEnvironmentVariableW(w "CODEX_PROXY_UPDATER_MODULES_PRESENT", p 0)'
  System::Call 'kernel32::SetEnvironmentVariableW(w "CODEX_PROXY_UPDATER_MODULES", p 0)'
  System::Call 'kernel32::SetEnvironmentVariableW(w "CODEX_PROXY_UPDATER_POLICY_PRESENT", p 0)'
  System::Call 'kernel32::SetEnvironmentVariableW(w "CODEX_PROXY_UPDATER_POLICY", p 0)'
!macroend

!macro WriteApplicationPlan Mode Quiet
  FileOpen $0 "$PLUGINSDIR\plan.ini" w
  FileWriteWord $0 0xFEFF
  FileClose $0
  WriteINIStr "$PLUGINSDIR\plan.ini" "Application" "Action" "${Mode}"
  WriteINIStr "$PLUGINSDIR\plan.ini" "Application" "InstallRoot" "$INSTDIR"
  WriteINIStr "$PLUGINSDIR\plan.ini" "Application" "PayloadZip" "$PLUGINSDIR\payload.zip"
  WriteINIStr "$PLUGINSDIR\plan.ini" "Application" "ZipSha256" "${ZIP_SHA256}"
  WriteINIStr "$PLUGINSDIR\plan.ini" "Application" "Version" "${APP_VERSION}"
  WriteINIStr "$PLUGINSDIR\plan.ini" "Application" "SourceCommit" "${SOURCE_COMMIT}"
  WriteINIStr "$PLUGINSDIR\plan.ini" "Application" "UninstallerPath" "$PLUGINSDIR\uninstall.exe"
  WriteINIStr "$PLUGINSDIR\plan.ini" "Application" "UpdateEnginePath" "$PLUGINSDIR\install_update.ps1"
  WriteINIStr "$PLUGINSDIR\plan.ini" "Application" "ResultPath" "$PLUGINSDIR\result.json"
  WriteINIStr "$PLUGINSDIR\plan.ini" "Application" "Silent" "${Quiet}"
  WriteINIStr "$PLUGINSDIR\plan.ini" "Application" "DesktopShortcut" "$DesktopChoice"
!macroend

Function DirectoryShow
  ${If} $RegisteredRoot != ""
    !insertmacro MUI_HEADER_TEXT "安装位置" "此版本将沿用已安装目录。更换位置前，请先卸载现有版本。"
  ${Else}
    !insertmacro MUI_HEADER_TEXT "安装位置" "默认安装到当前用户目录，也可选择其他本机目录或磁盘。"
  ${EndIf}
FunctionEnd

Function DirectoryLeave
  ${If} $RegisteredRoot != ""
    StrCmp $INSTDIR $RegisteredRoot directory_unchanged
      MessageBox MB_ICONEXCLAMATION "升级将沿用原安装目录。更换位置前，请先卸载现有版本。"
      Abort
directory_unchanged:
  ${EndIf}
FunctionEnd

Function IconOptionsShow
  !insertmacro MUI_HEADER_TEXT "图标选项" "选择常用入口。"
  nsDialogs::Create 1018
  Pop $OptionsDialog
  ${If} $OptionsDialog == error
    SetErrorLevel 2
    Abort
  ${EndIf}
  ${NSD_CreateLabel} 0 0 100% 30u "程序将安装到：$\r$\n$INSTDIR"
  Pop $0
  ${NSD_CreateCheckbox} 0 42u 100% 14u "创建桌面快捷方式"
  Pop $DesktopCheckbox
  ${NSD_SetState} $DesktopCheckbox $DesktopChoice
  ${NSD_OnClick} $DesktopCheckbox DesktopChoiceChanged
  ${NSD_CreateLabel} 0 66u 100% 30u "开始菜单快捷方式将始终创建。配置、历史和日志仍保存在当前用户的数据目录。"
  Pop $0
  nsDialogs::Show
FunctionEnd

Function DesktopChoiceChanged
  Pop $0
  ${NSD_GetState} $DesktopCheckbox $DesktopChoice
FunctionEnd

Function IconOptionsLeave
  ${NSD_GetState} $DesktopCheckbox $DesktopChoice
FunctionEnd

Function .onInit
  SetShellVarContext current
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "CodexProxyTray 需要 64 位 Windows。"
    SetErrorLevel 2
    Abort
  ${EndIf}
  ${IfNot} ${AtLeastWin10}
    MessageBox MB_ICONSTOP "CodexProxyTray 需要 Windows 10 或更新版本。"
    SetErrorLevel 2
    Abort
  ${EndIf}
  SetRegView 64
  ReadRegStr $RegisteredRoot HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexProxyTray" "InstallLocation"
  StrCpy $DesktopChoice 1
  ${If} $RegisteredRoot != ""
    StrCpy $DesktopChoice 0
    ClearErrors
    ReadRegDWORD $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexProxyTray" "DesktopShortcut"
    ${IfNot} ${Errors}
      ${If} $0 == 1
        StrCpy $DesktopChoice 1
      ${EndIf}
    ${EndIf}
  ${EndIf}
  ${GetParameters} $0
  ClearErrors
  ${GetOptions} $0 "/DESKTOP=" $1
  ${If} ${Errors}
    ClearErrors
    ${GetOptions} $0 "/DESKTOP" $1
    ${IfNot} ${Errors}
      SetErrorLevel 2
      IfSilent +2
        MessageBox MB_ICONSTOP "桌面图标参数无效，请使用 /DESKTOP=0 或 /DESKTOP=1。"
      Abort
    ${EndIf}
  ${Else}
    ${If} $1 != 0
    ${AndIf} $1 != 1
      SetErrorLevel 2
      IfSilent +2
        MessageBox MB_ICONSTOP "桌面图标参数无效，请使用 /DESKTOP=0 或 /DESKTOP=1。"
      Abort
    ${EndIf}
    StrCpy $DesktopChoice $1
  ${EndIf}
  StrCpy $SystemPowerShell "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
FunctionEnd

Function un.onInit
  SetShellVarContext current
  ${IfNot} ${RunningX64}
    SetErrorLevel 2
    Abort
  ${EndIf}
  SetRegView 64
  StrCpy $DesktopChoice 0
  StrCpy $SystemPowerShell "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
FunctionEnd

Section "安装"
  SetShellVarContext current
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File /oname=payload.zip "${RELEASE_DIR}\CodexProxyTray-v${APP_VERSION}-windows-x64.zip"
  File /oname=install_application.ps1 "${ENGINE_PATH}"
  File /oname=install_update.ps1 "${UPDATE_ENGINE_PATH}"
  WriteUninstaller "$PLUGINSDIR\uninstall.exe"
  !insertmacro CaptureOriginalEnvironment ""
  IfSilent install_quiet install_interactive
install_quiet:
  !insertmacro WriteApplicationPlan Install 1
  Goto install_plan_ready
install_interactive:
  !insertmacro WriteApplicationPlan Install 0
install_plan_ready:
  StrCpy $BackendCommand '"$SystemPowerShell" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\install_application.ps1" -PlanPath "$PLUGINSDIR\plan.ini"'
  StrCpy $BootstrapText "stage=plan-ready;launcher=sysnative64"
  Call BootstrapDiagnostic
  !insertmacro EmitLauncherCheck ""
  ; nsExec uses a hidden console. No TIMEOUT: never kill an installation transaction for quiet output.
  nsExec::ExecToStack $BackendCommand
  Pop $BackendExit
  Pop $BackendOutput
  !insertmacro EmitBackendResult ""
  StrCmp $BackendExit "error" backend_launch_failed
  StrCmp $BackendExit "timeout" backend_unexpected_timeout
  StrCmp $BackendExit "0" backend_return_valid
  StrCmp $BackendExit "2" backend_return_valid
  StrCmp $BackendExit "3" backend_return_valid
  Goto backend_invalid_return
backend_return_valid:
  ${If} $BackendExit != 0
    DetailPrint "安装未完成。完整结果保存在当前用户 CodexProxyTray 数据目录的 installation-result-v1.json。"
    SetErrorLevel $BackendExit
    IfSilent +2
      MessageBox MB_ICONEXCLAMATION "安装或收尾未完成。请查看当前用户 CodexProxyTray 数据目录中的 installation-result-v1.json；原程序、配置和恢复资料已保留。"
    Abort
  ${EndIf}
  SetErrorLevel 0
  Goto backend_finished
backend_launch_failed:
  SetErrorLevel 2
  IfSilent +2
    MessageBox MB_ICONSTOP "无法启动安装处理程序，未完成安装。"
  Abort
backend_unexpected_timeout:
  SetErrorLevel 2
  DetailPrint "安装处理程序返回超时状态，未将本次操作记为成功，请保留恢复资料。"
  Abort
backend_invalid_return:
  SetErrorLevel 2
  DetailPrint "安装处理程序异常退出或返回了未知状态，未将本次操作记为成功。"
  DetailPrint $BackendOutput
  Abort
backend_finished:
  !insertmacro ClearPrivateEnvironment
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File /oname=install_application.ps1 "${ENGINE_PATH}"
  File /oname=install_update.ps1 "${UPDATE_ENGINE_PATH}"
  !insertmacro CaptureOriginalEnvironment "un."
  !insertmacro WriteApplicationPlan Uninstall 1
  StrCpy $BackendCommand '"$SystemPowerShell" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\install_application.ps1" -PlanPath "$PLUGINSDIR\plan.ini"'
  StrCpy $BootstrapText "stage=plan-ready;launcher=sysnative64"
  Call un.BootstrapDiagnostic
  !insertmacro EmitLauncherCheck "un."
  nsExec::ExecToStack $BackendCommand
  Pop $BackendExit
  Pop $BackendOutput
  !insertmacro EmitBackendResult "un."
  StrCmp $BackendExit "error" uninstall_launch_failed
  StrCmp $BackendExit "timeout" uninstall_unexpected_timeout
  StrCmp $BackendExit "0" uninstall_return_valid
  StrCmp $BackendExit "2" uninstall_return_valid
  StrCmp $BackendExit "3" uninstall_return_valid
  Goto uninstall_invalid_return
uninstall_return_valid:
  ${If} $BackendExit != 0
    DetailPrint "卸载收尾未完成。用户数据和未确认的文件已保留。"
    SetErrorLevel $BackendExit
    IfSilent +2
      MessageBox MB_ICONEXCLAMATION "卸载或收尾未完成，未确认的文件已保留。请查看当前用户 CodexProxyTray 数据目录中的 installation-result-v1.json。"
    Abort
  ${EndIf}
  SetErrorLevel 0
  Goto uninstall_finished
uninstall_launch_failed:
  SetErrorLevel 2
  IfSilent +2
    MessageBox MB_ICONSTOP "无法启动卸载处理程序，原目录已保留。"
  Abort
uninstall_unexpected_timeout:
  SetErrorLevel 2
  DetailPrint "卸载处理程序返回超时状态，未将本次操作记为成功，请保留恢复资料。"
  Abort
uninstall_invalid_return:
  SetErrorLevel 2
  DetailPrint "卸载处理程序异常退出或返回了未知状态，未将本次操作记为成功。"
  DetailPrint $BackendOutput
  Abort
uninstall_finished:
  !insertmacro ClearPrivateEnvironment
SectionEnd
