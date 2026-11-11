# CodexProxyTray

**Codex 一直“重新连接（Reconnecting 1/5～5/5）”？浏览器能联网、代理也开着，ChatGPT / Codex 还是连不上？**

CodexProxyTray 为 ChatGPT 桌面端和 Windows Codex CLI 提供带代理的启动环境，同时保留桌面应用的 Windows 包身份。它针对代理没有正确传入目标进程、启动入口丢失包上下文等场景，集中处理启动条件、持续守护和连接诊断。

**只想让 ChatGPT / Codex 走代理，不想为它们开启 VPN / 代理客户端的 TUN 模式、影响其他软件？** 可以让代理客户端提供本地 HTTP / mixed 端口，再由 CodexProxyTray 将目标应用接入这个端口，无需为了这两个应用开启 TUN 或系统全局代理。

填好代理地址，开启守护，把窗口收起到托盘。之后目标应用通过 Windows 应用包激活链启动时会使用配置的环境；Windows `codex` 则通过专用命令入口接入，省去每次启动前手动设置环境变量的步骤。

**不修改 Windows 系统代理，也不写入全局 `HTTP_PROXY`、`HTTPS_PROXY`、`ALL_PROXY` 等代理变量。** 普通进程的代理环境保持原样。

> Windows x64 · 社区工具，与 OpenAI 无隶属关系。正式版本及下载文件见 [GitHub Releases](https://github.com/OrangeZh1/CodexProxyTray/releases)。

## 它解决什么问题

适合已经有可用本地 HTTP 代理，却仍遇到桌面应用重连、部分请求失败，或需要反复维护启动方式的用户。

| 遇到的痛点 | 当前版本如何处理 |
| --- | --- |
| 为了 ChatGPT / Codex 联网而开启 TUN 或系统全局代理，其他软件的网络也跟着受影响 | 通过现有代理客户端的本地 HTTP / mixed 端口，为目标应用提供独立代理环境；可以在不为它们开启 TUN / 系统全局代理的情况下使用。 |
| Codex 反复显示 `Reconnecting 1/5～5/5` | 为与代理传递或启动环境有关的重连问题提供带代理的入口，并通过检测和日志帮助定位。其他原因导致的重连仍需分别排查。 |
| 浏览器和代理客户端正常，桌面应用或子进程仍连不上 | 为目标启动进程设置 `HTTP_PROXY`、`HTTPS_PROXY`、`ALL_PROXY`，供遵循这些变量的进程及其后续子进程使用。 |
| 直接运行商店安装目录里的 EXE，丢失 Windows 应用包上下文 | 沿用 Windows 应用包激活链，核验目标进程的包身份后恢复启动，不把桌面应用作为无包身份的普通 EXE 拉起。 |
| 应用升级后安装目录变化，固定版本路径或旧快捷方式失效 | 按当前用户已安装的应用包识别目标，检测包版本变化并重新建立守护，避免把某个版本的安装路径写死。 |
| 每次启动都要设置代理变量、使用特定脚本或保持命令行窗口 | 桌面助手以托盘程序持续守护，关闭主窗口后继续运行；用户主动打开 Codex CLI 时，才使用系统终端运行 CLI。 |
| Windows `codex` 命令没有拿到指定代理 | 接入专用命令入口；助手运行且守护启用时，只向 CLI 子进程传入代理，转发原有参数和退出码。 |
| 代理没启动、端口填错，却只能看到应用连接失败 | 检测配置的本地 TCP 端口，分别展示代理、应用接入和网页探测状态；日志可按日期、级别与关键词筛选，并查看完整失败详情。 |
| 修改代理设置时，不希望结束正在进行的应用或任务 | 保留已运行进程，新配置作用于后续启动的目标进程；界面提示何时需要在保存工作后手动重开应用。 |

### 重连、“结束 WSL”和内置提示词异常的实际案例

[旧版 README](https://github.com/OrangeZh1/CodexProxyTray/blob/90110fe4f68d91036e198e389258795c8df3922d/README.md) 记录过这样的使用反馈：代理已经开启，但 Codex 仍反复显示“重新连接”，同时遇到**卡在“结束 WSL”以及内置提示词加载异常**；改用带代理、保留包身份的启动入口后，这些现象在当时的使用环境中恢复正常。

这个案例是项目的起因之一。软件直接处理的是**代理环境传递和 Windows 应用包启动上下文**；它不执行 `wsl --shutdown`，不修改 WSL 配置，也不读取、注入或修改内置提示词。这里记录的是观察到的改善，不代表已证明所有重连、WSL 或提示词问题都由同一原因引起。服务端故障、登录状态、不可用的代理节点或应用自身缺陷，仍需单独排查。

### 代理作用范围

- 代理变量作用于目标启动进程及继承环境的子进程，不写入用户或系统的全局代理环境。
- 软件不切换代理客户端的 TUN、系统代理或分流开关。如果你的客户端仍开启这些模式，其他软件仍受客户端原有规则影响。
- 同时设置 `NO_PROXY=localhost,127.0.0.1,::1`，让遵循该变量的本地请求绕过代理。
- 不修改目标应用的安装文件、账号、模型提供方或内置提示词，也不强制关闭 WebSocket。
- WSL、远程主机和容器内的代理需要分别配置；忽略代理环境变量的组件或已运行的后台进程，不会因此自动改用指定代理。

使用前先启动自己的代理客户端，准备一个可用的本地 HTTP 代理地址，例如 `http://127.0.0.1:7890`。本软件不提供代理节点，具体线路和分流规则由你的代理客户端负责。

## 快速上手

### 下载正式版本

打开 [最新正式 Release](https://github.com/OrangeZh1/CodexProxyTray/releases/latest)，下载其中的 Windows x64 发行 ZIP，例如 `CodexProxyTray-v0.1.0-windows-x64.zip`。这是解压运行的便携发行包，完整解压到固定目录后，双击 `CodexProxyTray.exe`。

GitHub 自动生成的 Source code 是源码归档；日常使用请下载上面的 Windows x64 发行包。Release 同时提供 `SHA256SUMS.txt`、版本说明和 `CodexProxyTray-update.json`。包内 [QUICKSTART.txt](QUICKSTART.txt) 提供离线中文说明。

请保留同目录的 `codex-proxy-cli.exe`、`WebView2Loader.dll` 和许可文件。电脑需安装 Microsoft Edge WebView2 Runtime，以及你要使用的 ChatGPT 桌面端或 Windows Codex CLI。

桌面助手本身无需 Node.js、Python 或 VBScript 运行环境，前端已内嵌到程序中。如果 Codex CLI 通过 npm 安装，它自身需要的 Node.js 环境仍应保留。

### 配置并开始使用

1. 启动本机代理客户端，确认 HTTP / mixed 端口可用；使用该端口接入目标应用时，无需特意开启 TUN 或系统全局代理。
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

应用包结构、Windows 接口或系统策略变化可能影响兼容性。遇到未识别应用或包身份验证失败时，请保留脱敏后的日志，不要删除身份校验或直接改为运行安装目录中的 EXE。

### 是否需要关闭或额外安装 WSL？

不需要。助手处理 Windows 端应用的启动环境，不会关闭 WSL，也不负责安装或修复 Linux 环境。需要 Linux 工具链、构建或测试时，可以继续按项目需要使用 WSL；Linux 内的代理仍需单独核对。

WSL 与 WS / WebSocket 是不同概念：前者是 Linux 开发环境，后者是网络通信协议。软件不通过关闭它们来处理连接问题。

### 代理地址怎么填？

填写本机 HTTP 或 mixed 代理端口，地址使用 `http://`，例如 `http://127.0.0.1:7890`、`http://localhost:7890` 或 `http://[::1]:7890`。SOCKS-only 端口、远程主机地址和带账号密码的代理 URL 不在当前支持范围内。

### 旧版的安装脚本和代理自动识别还适用吗？

旧版 `Install.cmd` / VBS、`-ProxyPort`、`-CheckOnly` 和安装时读取系统代理的自动识别流程属于历史脚本版。当前托盘版在“代理”页显式填写并保存地址，尚未提供相同的代理自动识别流程。请按本文的快速上手使用当前版本；保留的历史使用案例不表示旧安装命令仍适用于当前目录。

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
