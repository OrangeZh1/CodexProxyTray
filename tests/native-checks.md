# Windows 原生检查

从 ChatGPT / Codex 宿主启动的 PowerShell 可能处于 MSIX 包上下文。同样的 AppData 路径会读到虚拟化副本，不能作为独立桌面进程的状态证据。`Test-Native.ps1` 现在先核对自身包身份，检测到该上下文即停止，不改写 AppData。

需要真实本机 ChatGPT 安装的 `Test-Native.ps1` 通过 `Start-Independent.ps1` 执行。后者以当前会话同一用户的 Explorer 为父进程，使用桌面用户令牌生成的新环境，以暂停状态创建进程；确认没有包身份后才放行。检查进程隐藏运行，超时只收尾本次检查及其子进程。`Test-Native.ps1` 不会关闭 ChatGPT 或运行中的助手。

## GitHub Actions 隔离 CLI 检查

公开仓库的完整测试和构建按当前规则在 GitHub Actions 执行。Windows job 编译出实际 `codex-proxy-cli.exe` 后，用 PowerShell 7.2 以上版本和已准备的 Node.js 执行：

```powershell
./tests/Test-Cli.ps1 `
  -BinaryPath "$env:CARGO_TARGET_DIR/x86_64-pc-windows-msvc/release/codex-proxy-cli.exe" `
  -EvidenceDirectory "$env:RUNNER_TEMP/CodexProxyTray-cli-evidence"
```

`BinaryPath` 必须明确指定编译产物，脚本不会搜索或选择其他私有文件。可以用 `NodePath` 明确指定 `node.exe`。`EvidenceDirectory` 必须不存在；省略时会生成唯一证据目录并在成功输出中显示位置。上传 CI 证据时使用该目录中的 `cli-checks.json`。证据包含实际二进制 SHA256、真实执行的逐项结果、父进程与 User/Machine 环境未变化的核对结果，以及临时目录收尾状态。

脚本只在独立临时树中提供固定 mock npm CLI，实际执行编译后的 shim 与真实 Node.js。覆盖 9 个场景：无事件时的 `--proxy-info`、已有变量透传和参数/退出码、未设置变量透传、未触发事件、活动守护信息、活动守护覆盖及复杂参数/退出码、从未设置变量注入、普通 Node 子进程不受影响、事件复位后恢复透传。命名事件按当前应用的根目录小写 UTF-8 FNV-1a 64 位规则构造；不启动真实助手。

只有子进程的环境会被设置，继承环境先清空，再添加运行所需最小字段与固定 QA 变量；不写 User/Machine 环境或注册表，不读取真实 Codex 账号，不发起网络请求。mock 只输出固定代理白名单与原始参数，报告不保留完整环境或原始标准输出。超时只停止本次启动的进程树。

`TempRoot` 与证据目录必须是 OS Temp / `RUNNER_TEMP` 下的新目录，且位于源码目录之外。收尾前重新核对绝对路径、所有权标记、允许的文件清单与 reparse point；验证通过才删除自有沙箱。出现未知文件或路径变化时保留沙箱并使检查失败。证据目录保留供 CI 归档。

这套隔离检查不覆盖交互式 Ctrl+C、真实 Codex 登录或网络、ChatGPT 应用包启动/退出/升级、窗口和托盘 UI、实际登录自启。它不能代替下述真实本机验收。

## GitHub Actions 辅助服务检查

Windows job 还显式执行一个默认忽略的 ETW 回环检查：在隔离 runner 创建自有实时会话，先验证空目标列表不计入测试进程的回环活动，再绑定该进程身份，发送固定本机 TCP 数据并验证实际事件与字节增长，最后确认会话停止。它不访问外网或真实 ChatGPT；不能用纯解析单元测试替代这项结果。

`Test-TrafficService.ps1` 仅在明确已提权、没有同名服务和专属目录的隔离 Windows runner 上，使用指定的本轮 GUI 产物测试安装入口、SCM 配置、受保护文件的哈希与权限、真实 ServiceMain 启停、同构建重复安装和卸载。它还以独立测试配置启动实际助手后台，经辅助服务取得至少两次 ETW 采样，并在退出时确认 STOP 回执和采集器进程退出。

测试配置关闭外部联网、网页探测和自动更新。它不在用户电脑自动安装，不使用桌面自动化、截图或真实账号，也不启动真实 ChatGPT / Codex CLI。

```powershell
./tests/Test-TrafficService.ps1 -BinaryPath "<本轮便携目录>/CodexProxyTray.exe" -EvidenceDirectory "$env:RUNNER_TEMP/CodexProxyTray-service-evidence"
```

安装器的显式命令行入口要求当前进程已提权，并且 `--owner-sid` 与当前令牌一致，不自行弹 UAC；界面安装或卸载入口才调用 Windows 授权。证据只记录检查结果、状态和程序哈希，不导出 SID、完整环境或凭据。已有同名对象、未知文件或归属不匹配会使检查停止并保留现场，不能为取得通过而清理。

该检查不等于用户电脑已安装服务，也不能证明真实 ChatGPT / CLI 的实时 ETW 流量数值准确。真实目标应用的采集、Windows 授权取消以及进程按路由访问外部服务仍分别报告验证范围。

## 已有基线的验收

每次选择新的空证据目录，保留之前的失败与日志：

```powershell
.\tests\Start-Independent.ps1 `
  -CheckScript .\tests\Test-Native.ps1 `
  -EvidenceDirectory "$env:TEMP\CodexProxyTray-native-new-run" `
  -BaselinePath "$env:TEMP\CodexProxyTray-native-baseline.json"
```

基线不会覆盖；证据目录已有内容时拒绝重用。报告记录检查进程与助手的包身份、Job 状态、实际受检 exe 的 SHA256、两次目标包激活、普通进程隔离、新用户环境中的 CLI 解析与版本执行。代理变量只记录指纹。所有 JSON 显式按 UTF-8 读写。

## 新建基线

需要新基线时，先在本轮助手启动之前执行；已有基线继续保留：

```powershell
.\tests\Start-Independent.ps1 `
  -CheckScript .\tests\Test-Native.ps1 `
  -CaptureBaseline -BaselinePath "$env:TEMP\CodexProxyTray-baseline-new.json"
```

## 启动与单实例命令

```powershell
.\tests\Start-Independent.ps1 -Executable "C:\path\CodexProxyTray.exe"
.\tests\Start-Independent.ps1 -Executable "C:\path\CodexProxyTray.exe" -Background
.\tests\Start-Independent.ps1 -Executable "C:\path\CodexProxyTray.exe" -Action Refresh
.\tests\Start-Independent.ps1 -Executable "C:\path\CodexProxyTray.exe" -Action Quit
```

`Refresh` / `Quit` 只适用于已实现相应命令的新构建；不要对旧开发版假定支持。快速退出的第二实例会记录 `launcher_exited=true`，不会误把退出的启动进程当作仍在运行的助手。

原生脚本不执行 ChatGPT 主进程退出、重开、升级或系统登录验收，也不执行桌面 UI 自动化。报告明确保留 `desktop_restart_performed=false`；进程身份与包激活检查不能代替这些未执行场景。
