# cloudcode

> 自托管 LLM 网关：集中管理凭据，团队透明使用 Claude Code，每次请求留审计。

## 组件

- **`cloudcode-hub`** —— 中转层，做账号鉴权、ACL、JSONL 审计；公网部署
- **`cloudcode-agent`** —— 出站连 hub 的 WS 隧道端点，按订阅凭据转发请求；与日常 `claude /login` 同一系统账号下运行即可
- **`cloudcode`** —— 客户端 launcher，开发者本地启动 claude

## 架构

![cloudcode 架构](docs/architecture.svg)

源文件见 [`docs/architecture.drawio`](docs/architecture.drawio)（[diagrams.net](https://app.diagrams.net) 可编辑）。

## 安装

Hub（公网机器）：

```bash
curl -fsSL https://raw.githubusercontent.com/initialz/cloudcode/main/install.sh | sh -s -- hub
```

Agent（任意已 `claude /login` 的机器，NAT 后也可以）：

```bash
curl -fsSL https://raw.githubusercontent.com/initialz/cloudcode/main/install.sh | sh -s -- agent
```

Client（开发者本机）：

```bash
curl -fsSL https://raw.githubusercontent.com/initialz/cloudcode/main/install.sh | sh -s -- client
```

支持 Linux x86_64/aarch64、macOS aarch64。

## 使用

### Hub（管理员）

```bash
cloudcode-hub gen-token alice      # 为每个用户生成 token
$EDITOR ./hub.toml                 # 加 [[accounts]] 和 [[agents]]
cloudcode-hub serve --config ./hub.toml          # 前台运行，日志打到 stdout
# 或
cloudcode-hub daemon start --config ./hub.toml   # 后台运行
```

### Agent（一次性设置）

跟你日常 `claude /login` 同一个系统账号下运行就好，agent 会自己读 `~/.claude/.credentials.json`，不用复制文件或改权限。

```bash
cloudcode-agent gen-secret         # 一次性生成 shared_secret + 哈希
$EDITOR ./agent.toml               # 粘 [hub].url + [auth].shared_secret
cloudcode-agent serve --config ./agent.toml          # 前台运行，日志打到 stdout
# 或
cloudcode-agent daemon start --config ./agent.toml   # 后台运行
```

`gen-secret` 的输出里还包含 `[[agents]]` 段落 —— 把那段交给 hub 管理员加到 `hub.toml`。

### Client（开发者）

```toml
# ~/.config/cloudcode/config.toml
hub_url = "https://your-hub-host"
token   = "cc_xxx_from_admin"
```

```bash
cd ~/code/myproj
cloudcode run claude
```

体验和原生 `claude` 一致——所有 API 调用经 hub 鉴权、路由、留审计。

> Daemon 模式日志写到 `~/.local/state/cloudcode/{hub,agent}.log`，用 `cloudcode-hub|cloudcode-agent daemon {status,stop,restart}` 管理生命周期。

## 配置参考

[`hub.example.toml`](hub.example.toml) · [`agent.example.toml`](agent.example.toml)

> ⚠️ 共享订阅给多人使用违反 Anthropic ToS，建议每位用户绑自己的订阅（一人一个 agent）。

## License

MIT
