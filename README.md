# CodexProxyTray

**让 ChatGPT 桌面端和 Windows Codex CLI 使用你指定的本地 HTTP 代理。**

填好代理地址，开启守护，把窗口收起到托盘。助手为目标应用后续启动的进程提供代理环境，省去每次启动前手动设置环境变量的步骤。

**不修改 Windows 系统代理，也不写入全局 `HTTP_PROXY`、`HTTPS_PROXY`、`ALL_PROXY` 等代理变量。** 普通进程的代理环境保持原样。

> Windows x64 · 开发预览版。当前尚未发布 CodexProxyTray 正式安装包。

## 它解决什么问题

适合已经有可用本地 HTTP 代理，希望单独管理 ChatGPT 桌面端和 Windows Codex CLI 代理入口的用户。

| 重点 | 使用效果 |
| --- | --- |
| ChatGPT 桌面端专用代理 | 为目标应用包后续启动的进程提供代理环境，保留正在运行的应用和工作。 |
| Windows Codex CLI 接入 | 接入 `codex` 命令入口；助手运行且守护启用时，将指定代理传给 CLI 子进程。 |
| 托盘持续守护 | 关闭主窗口后继续运行，随时暂停、恢复，也可从托盘打开应用或重新检测。 |
| 看清连接问题 | 集中查看代理端口、网页探测、应用状态和完整日志，区分代理未启动、应用未接入与探测失败。 |

使用前先启动自己的代理客户端，准备一个可用的本地 HTTP 代理地址，例如 `http://127.0.0.1:7890`。本软件不提供代理节点，具体线路和分流规则由你的代理客户端负责。

## 快速上手

### 获取开发预览版

在 [GitHub Actions](https://github.com/OrangeZh1/CodexProxyTray/actions/workflows/ci.yml) 中选择最新一次成功的运行，下载 `CodexProxyTray-windows-x64` 制品，完整解压后运行 `CodexProxyTray.exe`。CI 制品限时保留，用于开发预览和验收。

请保留同目录的 `codex-proxy-cli.exe`、`WebView2Loader.dll` 和许可文件。电脑需安装 Microsoft Edge WebView2 Runtime，以及你要使用的 ChatGPT 桌面端或 Windows Codex CLI。

### 配置并开始使用

1. 启动本机代理客户端，确认 HTTP 代理端口可用。
2. 打开 CodexProxyTray，在“代理”页填入地址并点击“应用”。
3. 确认代理端口可用、对应应用已接入，并保持守护开启。
4. 打开 ChatGPT，或新开一个终端运行 `codex`；也可直接使用助手中的“打开 ChatGPT / Codex CLI”。
5. 关闭主窗口，让助手留在托盘继续运行。

**首次接入时注意：**

- 已经运行的 ChatGPT 保留原来的启动环境。保存工作后，在方便时手动完整退出并重新打开一次，新的代理环境才会生效。
- CLI 首次接入会把专用入口目录加入当前用户 PATH。请新开终端；直接调用原 CLI 的绝对路径会绕过专用入口。
- 登录自动启动和启动后自动打开应用默认关闭，可在设置中开启。

## 界面与功能

| 页面 | 可以做什么 |
| --- | --- |
| 仪表盘 | 查看代理连接耗时趋势、ChatGPT 运行状态、TCP 连接数、网页连通性、助手运行时长和最近事件；暂停或恢复守护。 |
| 代理 | 修改本地 HTTP 代理，查看 ChatGPT / Codex CLI 接入状态，打开对应应用。 |
| 运行日志 | 点击记录查看完整详情；按级别、关键词和日期范围组合筛选；支持今天、昨天、近 7 天及自选日期，一键重置。 |
| 设置 | 调整主题、登录自动启动、启动后打开的应用、CLI 工作目录、连通检测和可选流量统计；在“关于”中查看版本、更新和许可。 |

侧栏支持收起与展开；主题支持浅色、深色和跟随系统。页面切换、侧栏、弹窗和操作反馈带有动效，并响应系统的“减少动态效果”偏好。

日志保存在本机，最多保留最近 **200 条**。日期筛选只查询已保留的记录，页面会显示当前记录覆盖的日期范围。

## 可选：目标进程流量统计

在“设置”中安装流量辅助服务，再开启“记录网络流量”，即可查看已识别的 ChatGPT / Windows Codex CLI 进程树的上传、下载速率和累计字节。统计从开启时开始累计，默认关闭，并记住你的开关选择。

- 安装、升级或卸载辅助服务需要 Windows 授权；安装完成后，日常开关统计不再反复请求 UAC，主窗口保持普通权限。
- 服务按需启动，没有活动采集器时自动停止，不设置服务开机自启。
- 仅计数字节，不读取通信内容，也不保存连接地址或端口；不依赖某个代理客户端的接口。
- 可能漏掉短命进程，丢事件时会提示统计不完整。数值表示目标进程的网络活动，不能当作整张网卡的计费流量。

显示“暂不可用”时，先看辅助服务状态和错误说明。若提示与当前应用不匹配，请先卸载旧服务，再安装当前版本；拒绝授权或采样失败时，界面显示原因和 `—`，不会用 0 速率冒充有效数据。

## 数据与联网

- 配置和最近日志保存在 `%LOCALAPPDATA%\CodexProxyTray`，没有遥测或数据上报。
- 不记录或上报对话、提示词和用户 CLI 参数；启动参数原样转发给已安装的 CLI。
- 联网功能用于经当前代理探测 ChatGPT 网页，以及读取 GitHub 公开更新清单。网页探测不使用 Cookie。
- 可单独关闭网页探测，也可在“设置 → 关于”关闭助手联网检测；关闭后停止后续探测和更新请求，应用代理守护与本机 TCP 探测继续运行。
- 更新检查只显示版本和发布说明，不自动下载或安装。自动检查默认关闭，可设置 1 / 3 / 6 / 12 / 24 小时间隔。

## 常见问题

### 代理端口可用，为什么 ChatGPT 仍可能无法使用？

“端口可用”表示本地 TCP 端口能连接；“ChatGPT 网页连通性”表示助手经配置代理请求 `chatgpt.com/cdn-cgi/trace` 的结果。网页探测展示该次 HTTP 耗时、出口 IP 和国家代码，不验证登录或对话。

若代理客户端按进程分流，助手的探测路线可能与 ChatGPT 会话不同。还应确认旧的 ChatGPT 已完整退出并重新启动，再到运行日志查看对应详情。

### 暂停守护或退出助手，会结束正在运行的应用吗？

不会。已经运行的进程保留启动时的环境。暂停或退出后，新启动的 CLI 按原配置运行。

### 支持哪些环境？

当前支持 Windows x64，CLI 接入针对本机安装的 Windows Codex CLI，不修改 WSL、远程主机或容器中的命令。当前识别的桌面应用包为 `OpenAI.Codex_2p2nqsd0c76g0`，界面名称使用 ChatGPT。

## 开发与验收

<details>
<summary>展开开发、构建、验证和更新清单说明</summary>

### 开发

前端使用 React 19、Fluent UI React v9 和本地打包的 Iconify Fluent 图标。原生部分使用 Rust、Tauri 2 和 Windows API。

开发预览使用 Node.js 24.14 或以上版本；最终 Windows 程序不依赖 Node.js 运行前端。

```powershell
npm ci --ignore-scripts --no-audit --no-fund
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

### 验证

本仓库为 public，完整测试和构建默认通过 GitHub Actions。工作流仅提供手动触发，不自动在每次 push/PR 上跑全量：

```text
gh workflow run ci.yml --ref main
```

Linux job 执行 Rust 核心检查、JavaScript 交互与更新检查、前端构建、实际 JSX 的 SSR 检查，以及生产界面的客户端启动与交互检查。Windows job 编译 x64 MSVC 内嵌版本，检查配置边界、真实 ETW 回环、隔离 CLI、辅助服务完整启停链路，以及真实主窗口和托盘 WebView 的启动与遮挡情况，再保存验收证据和便携制品。CI 不使用真实账号、业务数据库、密钥或实例材料，也不自动创建 GitHub Release。

GitHub CI 不可用时明确记录缺口，不将未运行的检查写为通过。真实 ChatGPT 应用包、本机终端与托盘的专项运行验收不使用账号模拟替代。

Windows 原生检查先在助手未启动时捕获代理环境基线，再启动助手，使用新的空证据目录运行：

```powershell
.\tests\Start-Independent.ps1 -CheckScript .\tests\Test-Native.ps1 -CaptureBaseline -BaselinePath "$env:TEMP\CodexProxyTray-proxy-baseline.json"
# 启动本轮构建的 CodexProxyTray，等待守护就绪。
.\tests\Start-Independent.ps1 -CheckScript .\tests\Test-Native.ps1 -EvidenceDirectory "$env:TEMP\CodexProxyTray-native-new-run" -BaselinePath "$env:TEMP\CodexProxyTray-proxy-baseline.json"
```

基线和证据路径每次使用新名称，脚本拒绝覆盖旧记录。只保存代理变量指纹，不导出完整环境或凭据。检查包括连续两次包内启动、普通子进程环境隔离，以及通过 `CreateEnvironmentBlock` 获取新用户环境，确认 `Get-Command codex` 解析到专用入口并实际执行版本命令。旧终端的 PATH 不会被这些检查修改。

原生脚本必须运行在独立桌面环境，避免 MSIX 宿主的 AppData 虚拟化导致读到旧状态；细节见 [Windows 原生检查](tests/native-checks.md)。托盘、窗口关闭后常驻、真实交互与退出恢复分别验证。SSR 检查不能替代视觉或真实点击验收，脚本也不会关闭当前 ChatGPT。

### 更新清单

更新检查从 GitHub Releases 的公开下载入口读取 `CodexProxyTray-update.json`，不使用共享 IP 易触发限流的 REST API，也不需要令牌。成功结果缓存 15 分钟，失败缓存 1 分钟；错误只显示可读提示。仓库改名后保留 GitHub 重定向。

新版本必须随正式 Release 提供如下清单，并将对应 Release 设为 latest：`{"schema":1,"product":"CodexProxyTray","version":"0.1.0","notes":"发行说明"}`。更新检查同时校验产品标识，仅接受三个数字组成的正式版本号；历史启动器的版本号不会直接用于本产品的升级比较。缺少清单时显示“暂无正式版本”。CI 制品用于验收，正式 Release 需要独立发布。

### 辅助服务权限

辅助服务使用 Windows LocalSystem 账户提供 ETW 所需权限，程序复制到受保护的 Program Files 目录，不从用户可写的源码或便携目录注册特权服务。仅接受安装者账户经本机受控 IPC 发出的固定采集请求，不提供任意命令或路径执行能力。ETW 覆盖 TCP/UDP 的 IPv4/IPv6 发送与接收事件。

</details>

## 许可

项目采用 [MIT 许可](LICENSE)。界面图标来自本地打包的 Iconify Fluent 集合，不连接图标 CDN。应用 Logo 的矢量源为 `ui/icon.svg`，图标导出脚本为 `scripts/Export-Icons.ps1`。
