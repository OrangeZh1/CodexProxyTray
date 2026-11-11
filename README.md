# CodexProxyTray

面向 Windows x64 的 Tauri 托盘程序，为 ChatGPT 桌面端和 Windows Codex CLI 提供专用代理启动环境。

当前为开发版本，尚未发布 CodexProxyTray 正式安装包。

## 功能

- 启动助手后开启守护，关闭窗口收起到托盘。
- 为 ChatGPT 应用包的后续启动提供代理环境；不结束正在运行的桌面应用。
- 接入 Windows `codex` 命令入口。助手运行且守护启用时，代理变量仅传给 CLI 子进程。
- 仪表盘展示本地代理连接耗时、ChatGPT 网页探测、TCP 连接、本次守护时长与工作集；右下角可暂停或恢复守护。
- 可选的 Windows ETW 进程网络统计，配有本机辅助服务的安装、状态检查和卸载入口；仅归属已识别的 ChatGPT / Windows Codex CLI 进程树，不依赖代理客户端接口。
- 配置本地 HTTP 代理地址，分别查看代理端口、桌面应用、CLI 的状态。
- 设置页分为常规与关于：配置登录自动启动、启动后打开 ChatGPT / Codex CLI / 两者，偏好即时保存；CLI 可设置工作目录，关于页内查看版本、更新与许可。
- 从页面或托盘打开 ChatGPT、Codex CLI。
- 手动检查 GitHub Releases，显示新版本、更新说明和发布页入口；可关闭更新联网，自动检查支持 1/3/6/12/24 小时间隔。
- 一体化窗口顶部、可收放侧栏、分组托盘菜单、单实例运行；主题支持浅色、深色和跟随系统。
- 分类日志包含检测结果、耗时、进程与失败详情，保留最近 200 条，并在本机持续保存。

## 使用边界

- 配置的本地 HTTP 代理需要正常运行；助手不提供代理节点，也不绑定具体代理软件。
- 本机需有 Microsoft Edge WebView2 Runtime。仪表盘中的耗时表示本地 TCP 连接耗时，不代表外部服务响应时间；TCP 连接数只统计 ChatGPT 应用进程树中的已建立连接。
- “ChatGPT 网页连通性”通过当前配置的代理请求 `chatgpt.com/cdn-cgi/trace`，展示该次 HTTP 探测耗时、出口 IP 和国家代码。代理客户端的进程分流可能使它与 ChatGPT 会话路线不同；HTTP 成功不等于登录或对话可用。可单独关闭探测，关闭助手联网也会停止后续探测和更新请求。
- 流量统计默认关闭，从开启时开始累计，并记住“记录网络流量”的选择。首次安装辅助服务以及重新安装、卸载需要 Windows 授权；服务安装后，日常开启统计不再反复请求 UAC，普通主界面保持原权限。拒绝授权或采样不可用时显示明确状态，不以 0 速率替代。辅助服务与当前应用不匹配时，先卸载再重新安装。
- 辅助服务使用 Windows LocalSystem 账户提供 ETW 所需权限，程序复制到受保护的 Program Files 目录，不从用户可写的源码或便携目录注册特权服务。服务按需启动，不设置开机自启；没有活动采集器时自动停止。只接受安装者账户经本机受控 IPC 发出的固定采集请求，不提供任意命令或路径执行能力。
- ETW 仅读取目标 PID 与传输字节，覆盖 TCP/UDP 的 IPv4/IPv6 发送/接收事件，不保存包内容、连接地址或端口。进程名单周期刷新，可能漏掉短命进程；丢事件会报告不完整状态。数值表示目标进程网络活动，不能当作物理网卡或计费流量总量。
- 启用守护前已经启动的 ChatGPT，需要在方便时完整退出并重新打开一次。
- CLI 首次接入会把专用入口目录加入当前用户 PATH。请新开终端后运行 `codex`；直接调用原 CLI 的绝对路径会绕过该入口。
- CLI 接入目标是当前电脑安装的 Windows CLI，不修改 WSL、远程主机或容器内的命令。
- 暂停或退出助手后，新启动的 CLI 按原配置运行；已经运行的进程保留启动时的环境。
- 开机自启和自动打开 ChatGPT 默认关闭。代理设置保存在 `%LOCALAPPDATA%\CodexProxyTray`。
- 当前桌面应用的 Windows 包标识仍为 `OpenAI.Codex_2p2nqsd0c76g0`，界面名称使用 ChatGPT。

## 开发

前端使用 React 19、Fluent UI React v9 和本地打包的 Iconify Fluent 图标。原生部分使用 Rust、Tauri 2 和 Windows API。

开发预览使用 Node.js 24.14 或以上版本；最终 Windows 程序不依赖 Node.js 运行前端。

```powershell
npm install
npm run build:ui
npm run check:ui
npm run preview
npm run tauri dev
```

`preview` 监听 `127.0.0.1:4173`，读取 `dist/`；修改 `ui/` 后重新执行 `build:ui`，预览自动刷新。普通浏览器只读本机状态；Tauri 窗口使用真实 IPC 后台，支持实际操作。

Windows 常规构建需按 Tauri 官方要求准备 Rust 与 C++ 工具链。生产构建将页面内嵌，不依赖预览服务器：

```powershell
npm run build
```

便携目录必须同时包含 `CodexProxyTray.exe`、`codex-proxy-cli.exe` 和 x64 `WebView2Loader.dll`。本机 GNU 构建的 loader 来自锁定依赖 `webview2-com-sys 0.39.1` 的 `x64/WebView2Loader.dll`，不能只复制两个 exe。

`scripts/package-windows.mjs --bin-dir <release目录> --loader <已校验DLL> --out <新的空目录>` 会先校验三份 PE 均为 x64、loader 的 SHA256 与已验证依赖一致，再生成便携目录和校验回执。`custom-protocol` 生产构建内嵌 `dist/`；devUrl 开发版不能当作独立交付包。

## 验证

本仓库为 public，完整测试和构建默认通过 GitHub Actions。工作流仅提供手动触发，不自动在每次 push/PR 上跑全量：

```text
gh workflow run ci.yml --ref main
```

Linux job 执行 Rust 核心检查、JavaScript 状态/更新/采样检查、前端构建及实际 JSX 的 SSR 结构检查；Windows job 编译 x64 MSVC 内嵌版本，验证隔离 CLI 的环境、参数和退出码，再保存仅含必要运行文件与校验回执的临时 CI 制品。CI 不使用真实账号、业务数据库、密钥或实例材料，也不自动创建 GitHub Release。

GitHub CI 不可用时明确记录缺口，不将未运行的检查写为通过。真实 ChatGPT 应用包、本机终端与托盘的专项运行验收不使用账号模拟替代。

Windows 原生检查先在助手未启动时捕获代理环境基线，再启动助手，使用新的空证据目录运行：

```powershell
.\tests\Start-Independent.ps1 -CheckScript .\tests\Test-Native.ps1 -CaptureBaseline -BaselinePath "$env:TEMP\CodexProxyTray-proxy-baseline.json"
# 启动本轮构建的 CodexProxyTray，等待守护就绪。
.\tests\Start-Independent.ps1 -CheckScript .\tests\Test-Native.ps1 -EvidenceDirectory "$env:TEMP\CodexProxyTray-native-new-run" -BaselinePath "$env:TEMP\CodexProxyTray-proxy-baseline.json"
```

基线和证据路径每次使用新名称，脚本拒绝覆盖旧记录。只保存代理变量指纹，不导出完整环境或凭据。检查包括连续两次包内启动、普通子进程环境隔离，以及通过 `CreateEnvironmentBlock` 获取新用户环境，确认 `Get-Command codex` 解析到专用入口并实际执行版本命令。旧终端的 PATH 不会被这些检查修改。

原生脚本必须运行在独立桌面环境，避免 MSIX 宿主的 AppData 虚拟化导致读到旧状态；细节见 `tests/native-checks.md`。托盘、窗口关闭后常驻、真实交互与退出恢复分别验证。SSR 检查不能替代视觉或真实点击验收，脚本也不会关闭当前 ChatGPT。

## 更新与许可

更新检查从 GitHub Releases 的公开下载入口读取 `CodexProxyTray-update.json`，不使用共享 IP 易触发限流的 REST API，也不需要令牌。成功结果缓存 15 分钟，失败缓存 1 分钟；错误只显示可读提示。仓库改名后保留 GitHub 重定向。

新版本必须随正式 Release 提供如下清单，并将对应 Release 设为 latest：`{"schema":1,"product":"CodexProxyTray","version":"0.1.0","notes":"发行说明"}`。仅接受三个数字组成的正式版本号。历史 `chatgpt-proxy-launcher` v1.0.0 没有新产品清单，因此不会被当作 CodexProxyTray 升级；缺少清单显示“暂无正式版本”。CI 制品用于验收，正式 Release 需要独立发布。

项目沿用 MIT 许可。界面图标统一来自 Iconify 的 Fluent 集合，随构建内嵌，不连接图标 CDN。应用 Logo 的矢量源为 `ui/icon.svg`，图标导出脚本为 `scripts/Export-Icons.ps1`。
