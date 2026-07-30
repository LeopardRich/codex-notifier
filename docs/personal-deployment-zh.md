# Windows、macOS 与 Ubuntu 个人部署

本流程只适用于由同一用户拥有和管理的设备。它使用绿色 `main` CI 生成的未签名
Windows 工程包、ad-hoc 签名 macOS 工程包和 Linux 中继包。这些不是公开发布产物；
不要转发给其他人，也不要把系统警告视为已满足生产信任。

## 1. 下载并校验 bundle

在 GitHub Actions 中打开 `main` 最新的绿色 `CI` 运行，下载
`codex-notifier-release-bundle` artifact，并解压到空目录。它包含：

- `codex-notifier-v0.1.0-windows-x86_64.zip`；
- `codex-notifier-v0.1.0-macos-universal.zip`；
- Linux x86-64 与 AArch64 中继归档；
- `SHA256SUMS`、SBOM、许可证材料和工程发布说明。

每台设备都必须先校验将要使用的归档。Ubuntu 使用：

```bash
sha256sum -c SHA256SUMS
```

macOS 使用：

```bash
shasum -a 256 -c SHA256SUMS
```

Windows 使用 PowerShell 计算所选归档的哈希，并与 `SHA256SUMS` 对照：

```powershell
(Get-FileHash .\codex-notifier-v0.1.0-windows-x86_64.zip -Algorithm SHA256).Hash.ToLowerInvariant()
```

每个归档内的 metadata 必须指向该 CI 运行相同的 40 字符 commit。保留外部解压
目录；Windows 卸载必须从这份外部副本执行。

## 2. 安装 Windows 桌面端

解压 Windows 归档，在普通交互用户会话中运行：

```powershell
.\codex-notifier.exe install --codex-version 0.144.5
.\codex-notifier.exe status --format json
.\codex-notifier.exe test task-completed --format json --wait-ms 60000
.\codex-notifier.exe test approval-requested --format json --wait-ms 60000
```

只有当 `status` 报告 `agent_running=true`、`notification="ready"`，且两个
`test` 都报告 `delivery="delivered"` 时才继续。Windows 可能显示未签名应用或
SmartScreen 警告；只对上一步已核对 SHA-256 的归档绕过警告。

这些自测不会授予 Codex hook 信任。打开一个交互式 Codex 会话，运行 `/hooks`，
检查新 `Stop` handler 中精确的已安装可执行文件和固定参数，然后信任该定义。
不要把 `--dangerously-bypass-hook-trust` 写入日常启动方式。

## 3. 安装 macOS 桌面端

把 macOS 归档复制到 Mac，校验后用 `ditto` 解压并检查 bundle 签名结构：

```bash
mkdir -p "$HOME/Downloads/codex-notifier-personal"
ditto -x -k codex-notifier-v0.1.0-macos-universal.zip \
  "$HOME/Downloads/codex-notifier-personal"
cd "$HOME/Downloads/codex-notifier-personal"
codesign --verify --deep --strict --verbose=2 "Codex Notifier.app"
```

该 app 只有 ad-hoc 工程签名，没有公证票据。仅在归档校验和签名结构均正确，并且
macOS 确实保留了下载 quarantine 标记时，才对这一 app 移除标记：

```bash
xattr -dr com.apple.quarantine "Codex Notifier.app"
```

通过 app 内的可执行文件安装并测试：

```bash
bin="Codex Notifier.app/Contents/MacOS/codex-notifier"
"$bin" install --codex-version 0.144.5
"$bin" status --format json
"$bin" test task-completed --format json --wait-ms 60000
"$bin" test approval-requested --format json --wait-ms 60000
```

在 macOS 提示时允许通知。确认 agent 正在运行、`notification="ready"`，且两个
自测均已投递后，再配置远程路径。在交互式 Codex 会话中运行 `/hooks`，检查已安装
app 可执行文件和固定参数，并信任精确的新 `Stop` handler。

## 4. 选择一个桌面作为 SSH 接收端

一个 Ubuntu relay 同一时间只指向一个桌面目标。选择 Windows 或 macOS，确认
Ubuntu 能通过局域网或私有 VPN 访问它，并启用该桌面的系统 OpenSSH Server。不要
只为本工具把 SSH 直接暴露到公网。

在 Ubuntu 创建专用密钥：

```bash
install -d -m 700 "$HOME/.ssh"
ssh-keygen -t ed25519 -f "$HOME/.ssh/codex-notifier-desktop" \
  -C codex-notifier-relay
```

解压后的 Ubuntu 中继归档已经包含本步骤所需模板。只把 `.pub` 公钥行安装到所选
桌面，并使用对应的强制命令模板：

- Windows：`examples/authorized_keys-windows.example`；
- macOS：`examples/authorized_keys-macos.example`。

替换 `USERNAME` 与 `DEDICATED_PUBLIC_KEY`。SSH 登录用户必须与运行桌面
agent 的操作系统用户相同。严格按照 [`restricted-ssh.md`](restricted-ssh.md)
设置 Windows ACL 或 Unix `0700`/`0600` 权限。

在桌面本机读取 SSH host key 指纹，通过可信渠道与 Ubuntu 扫描结果核对，确认后再
写入 Ubuntu 的 `~/.ssh/known_hosts`。把 `examples/ssh-config.example` 复制为
`~/.ssh/config` 中的独立 Host block，替换所有大写占位符，并保留：

```text
StrictHostKeyChecking yes
IdentitiesOnly yes
RequestTTY no
ClearAllForwardings yes
```

## 5. 安装 Ubuntu relay 与 Codex hook

用 `uname -m` 选择归档：`x86_64` 使用 x86-64 包，`aarch64` 或 `arm64`
使用 AArch64 包。解压后，先创建 relay 配置：

```bash
mkdir -p "$HOME/.config/codex-notifier"
cp examples/config.toml.example "$HOME/.config/codex-notifier/config.toml"
```

示例中的 SSH alias 是 `codex-notifier-desktop`，与 SSH 示例一致；若要改名，两个
位置必须一起修改。然后安装二进制、systemd 用户服务和已验证的 Codex 0.144.5
任务完成 hook：

```bash
./install.sh --codex-version 0.144.5
systemctl --user status codex-notifier.service
codex-notifier doctor ssh
codex-notifier status --format json
```

在 Ubuntu 打开一个交互式 Codex 会话，运行 `/hooks`，检查已安装可执行文件和
固定参数，并信任精确的新 `Stop` handler；Codex 会跳过未信任的用户 hook。配置
不存在时，`install.sh` 会拒绝启用服务或安装 hook；已有配置和无关 hook 会被
保留。继续之前，确认 `status` 报告 `role="relay"`、`installed=true` 与
`agent_running=true`。

如果退出 SSH 登录后 systemd 用户服务不会继续运行，可在明确接受常驻用户服务后
启用 lingering：

```bash
loginctl enable-linger "$USER"
```

## 6. 验证远程路径和真实 Codex 事件

在 Ubuntu 运行两条显式远程自测：

```bash
codex-notifier test task-completed --format json --wait-ms 60000
codex-notifier test approval-requested --format json --wait-ms 60000
```

两者都必须报告 `route="remote"` 与 `delivery="delivered"`，且所选桌面必须
显示原生通知。随后在 Ubuntu 运行一个普通 Codex 0.144.5 任务；其 `Stop` hook
应在无需手工执行 notifier 命令的情况下产生任务完成通知。

普通 Codex CLI 的 `PermissionRequest` hook 尚未通过 fixture 验证，因此不会
自动安装。显式 approval 自测只能证明通知路由可用，不代表 CLI 审批事件已经自动
接入。

在 Windows 与 macOS 之间切换目标时，先停止 relay，替换配置 alias 背后的 SSH
Host block 和已验证 host-key 条目，运行 `codex-notifier doctor ssh`，再重启
服务。不要为了方便改成 `StrictHostKeyChecking accept-new`。

## 7. 可逆卸载

Ubuntu 在保留的 relay 解压目录中运行：

```bash
./uninstall.sh
```

它会移除精确自有的 Codex hook、systemd unit 和已安装二进制，同时保留 relay
配置、无关 hook 与 SQLite 状态。确认不再需要远程投递后，再分别删除专用密钥、
SSH Host block、known-host 条目和桌面公钥行。

Windows 从保留的外部归档运行：

```powershell
.\codex-notifier.exe uninstall
```

macOS 通过保留的外部 app 可执行文件运行 `uninstall`。两个桌面端都按设计保留
SQLite 状态。
