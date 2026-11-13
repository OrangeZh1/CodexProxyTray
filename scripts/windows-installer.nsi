Unicode true
RequestExecutionLevel user
SetCompressor /SOLID lzma
Name "CodexProxyTray"
OutFile "${OUTPUT_FILE}"
InstallDir "$LOCALAPPDATA\Programs\CodexProxyTray"
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
!define MUI_ICON "${ICON_PATH}"
!define MUI_UNICON "${ICON_PATH}"
!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TEXT "将为当前 Windows 用户安装 CodexProxyTray。配置、历史和日志保存在程序目录之外。可选流量辅助服务独立管理，本安装器不请求其管理员授权。"
!define MUI_FINISHPAGE_RUN "$INSTDIR\current\CodexProxyTray.exe"
!define MUI_FINISHPAGE_RUN_TEXT "运行 CodexProxyTray"
!define MUI_UNCONFIRMPAGE_TEXT_TOP "将移除当前用户的 CodexProxyTray、开始菜单和软件列表入口。配置、历史及日志会保留。可选流量辅助服务独立管理。"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"

Var BackendExit
Var BackendCommand
Var BackendOutput
Var SystemPowerShell

!macro CaptureEnvironment Original Present Value
  System::Call 'kernel32::SetLastError(i 0)'
  System::Call 'kernel32::GetEnvironmentVariableW(w "${Original}", p 0, i 0) i.r0'
  ${If} $0 == 0
    System::Call 'kernel32::GetLastError() i.r1'
    ${If} $1 == 203
      System::Call 'kernel32::SetEnvironmentVariableW(w "${Present}", w "0") i.r3'
      System::Call 'kernel32::SetEnvironmentVariableW(w "${Value}", p 0)'
    ${ElseIf} $1 == 0
      System::Call 'kernel32::SetEnvironmentVariableW(w "${Present}", w "1") i.r3'
      System::Call 'kernel32::SetEnvironmentVariableW(w "${Value}", w "")'
    ${Else}
      SetErrorLevel 2
      Abort
    ${EndIf}
  ${Else}
    ${If} $0 > 32768
      SetErrorLevel 2
      Abort
    ${EndIf}
    IntOp $1 $0 * 2
    System::Alloc $1
    Pop $2
    ${If} $2 == 0
      SetErrorLevel 2
      Abort
    ${EndIf}
    System::Call 'kernel32::SetLastError(i 0)'
    System::Call 'kernel32::GetEnvironmentVariableW(w "${Original}", p r2, i r0) i.r3'
    System::Call 'kernel32::GetLastError() i.r1'
    ${If} $3 >= $0
    ${OrIf} $1 != 0
      System::Free $2
      SetErrorLevel 2
      Abort
    ${EndIf}
    System::Call 'kernel32::SetEnvironmentVariableW(w "${Value}", p r2) i.r3'
    System::Free $2
    ${If} $3 == 0
      SetErrorLevel 2
      Abort
    ${EndIf}
    System::Call 'kernel32::SetEnvironmentVariableW(w "${Present}", w "1") i.r3'
  ${EndIf}
  ${If} $3 == 0
    SetErrorLevel 2
    Abort
  ${EndIf}
!macroend

!macro CaptureOriginalEnvironment
  !insertmacro CaptureEnvironment PSModulePath CODEX_PROXY_UPDATER_MODULES_PRESENT CODEX_PROXY_UPDATER_MODULES
  !insertmacro CaptureEnvironment PSExecutionPolicyPreference CODEX_PROXY_UPDATER_POLICY_PRESENT CODEX_PROXY_UPDATER_POLICY
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
!macroend

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
  StrCpy $SystemPowerShell "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
FunctionEnd

Function un.onInit
  SetShellVarContext current
  ${IfNot} ${RunningX64}
    SetErrorLevel 2
    Abort
  ${EndIf}
  StrCmp $INSTDIR "$LOCALAPPDATA\Programs\CodexProxyTray" +4
    MessageBox MB_ICONSTOP "卸载目录与当前用户的固定安装目录不一致，未删除任何文件。"
    SetErrorLevel 2
    Abort
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
  !insertmacro CaptureOriginalEnvironment
  IfSilent install_quiet install_interactive
install_quiet:
  !insertmacro WriteApplicationPlan Install 1
  Goto install_plan_ready
install_interactive:
  !insertmacro WriteApplicationPlan Install 0
install_plan_ready:
  StrCpy $BackendCommand '$"$SystemPowerShell$" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $"$PLUGINSDIR\install_application.ps1$" -PlanPath $"$PLUGINSDIR\plan.ini$"'
  ; nsExec uses a hidden console. No TIMEOUT: never kill an installation transaction for quiet output.
  nsExec::ExecToStack $BackendCommand
  Pop $BackendExit
  Pop $BackendOutput
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
  !insertmacro CaptureOriginalEnvironment
  !insertmacro WriteApplicationPlan Uninstall 1
  StrCpy $BackendCommand '$"$SystemPowerShell$" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $"$PLUGINSDIR\install_application.ps1$" -PlanPath $"$PLUGINSDIR\plan.ini$"'
  nsExec::ExecToStack $BackendCommand
  Pop $BackendExit
  Pop $BackendOutput
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
