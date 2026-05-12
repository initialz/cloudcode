# cloudcode

> Remote-control your own `claude` from anywhere. A TUI chat client talks to a hub, which forwards every prompt to an agent on a machine that has already done `claude /login`.

## Intended use & disclaimer

**This project is only for remotely controlling _your own_ coding CLI.** Typical setup: you `claude /login`'d on a workstation / server / home box, and you want to drive `claude` from a laptop, a phone, or an SSH terminal that doesn't have your subscription credentials.

**Do not share a subscription account across multiple people.** Subscription plans (Claude Max / Pro) are issued per individual under the provider's Terms of Service. Routing many humans' prompts onto one subscription violates those terms. The recommended topology is **one user → one subscription → one agent**.

If you use this software to violate any provider's Terms of Service or applicable laws, **you are solely responsible for the consequences**. The authors and contributors provide this software as-is, with no warranty, and accept no liability for your usage.

## Components

- **`cloudcode-hub`** — public-facing gateway: account-token auth, workspace mutex, JSONL audit log. Relays PTY traffic between clients and agents.
- **`cloudcode-agent`** — long-running daemon on a host where you've `claude /login`'d. Requires `tmux`. Dials out to the hub over WSS. When the hub asks for a session, the agent spawns `tmux new -A -s cloudcode-<workspace> -c <cwd> claude` and pipes the PTY master to/from the hub. tmux session **persists across reconnects** — close `cloudcode` and reopen later, you're back where you left off.
- **`cloudcode`** — relay client on your laptop. First shows a small workspace picker for your agent; once you pick one, your terminal becomes the **native claude TUI** (status bar, todo board, diffs, permission prompts, claude's own `/clear`/`/login`/etc — everything). When claude exits you're dropped back at the picker.

## Features

- **Native `claude` TUI, not a web rewrap.** Your terminal becomes the real claude binary's interface — status bar, todo board, diffs, permission prompts, slash commands, all of it. Byte-perfect PTY passthrough, so DA1/DA2, mouse, and arbitrary escape sequences reach the remote process unchanged.
- **Workspace state survives the client.** Close cloudcode (or shut your laptop's lid) and the agent's tmux + claude keep running on the host. Reconnect later and you're back at the same prompt, same todo board, same in-progress agentic loop. Long-running tasks finish whether you're watching or not.
- **Hop between terminals freely.** Opening a workspace already attached from another machine evicts the old client back to its menu and the new one takes over the live tmux session — claude state preserved. No "workspace busy" deadlock.
- **macOS Seatbelt workspace sandbox (opt-in).** Each spawned `claude` is wrapped in a kernel-enforced sandbox: writes scoped to the active workspace, cross-user and cross-workspace reads denied, secrets (`~/.ssh`, `~/.aws`, `~/.gnupg`, TCC database) hidden, persistence vectors (shell init files, `~/Library/LaunchAgents`, `.git/hooks/`) blocked, camera + microphone explicitly denied. Network stays open. Linux support is on the roadmap.
- **Pass any `claude` argument straight through.** `cloudcode -- --continue`, `cloudcode -- --model opus`, `cloudcode -- "fix this bug"` — everything after `--` is forwarded verbatim to the spawned claude.
- **NAT-friendly architecture.** Agents dial out to the hub over WSS; no inbound ports required on the agent host. Park an agent on your home box behind a router and drive it from a hotel laptop.
- **One hub, many agents, many accounts.** A single hub routes multiple agents (one per host) and multiple users (one account-token per developer). Workspaces are namespaced per-account; the sandbox enforces it.
- **Audit-friendly.** Every session's PTY output is captured as an asciinema cast on the agent. Replay with `asciinema play <file>`. Keystrokes are deliberately not recorded to avoid pasted-token leakage.

## Architecture

![cloudcode architecture](docs/architecture.svg)

Source: [`docs/architecture.drawio`](docs/architecture.drawio) (open with [diagrams.net](https://app.diagrams.net)).

## Install

### Option A — Prebuilt binary

Hub (public host):

```bash
curl -fsSL https://raw.githubusercontent.com/initialz/cloudcode/main/install.sh | sh -s -- hub
```

Agent (any host where you've run `claude /login`; behind NAT is fine):

```bash
curl -fsSL https://raw.githubusercontent.com/initialz/cloudcode/main/install.sh | sh -s -- agent
```

Client (laptop):

```bash
curl -fsSL https://raw.githubusercontent.com/initialz/cloudcode/main/install.sh | sh -s -- client
```

Supported: Linux x86_64 / aarch64, macOS aarch64.

### Option B — Build from source

```bash
git clone https://github.com/initialz/cloudcode.git
cd cloudcode
cargo build --release --workspace

sudo install -m 0755 target/release/cloudcode-hub   /usr/local/bin/
sudo install -m 0755 target/release/cloudcode-agent /usr/local/bin/
sudo install -m 0755 target/release/cloudcode       /usr/local/bin/
```

## Usage

### Hub (administrator)

```bash
cloudcode-hub --init                     # writes ./hub.toml AND prints the
                                         # one-time agent registration token
                                         # — save it, you'll give it to every
                                         # agent operator
cloudcode-hub gen-token alice            # one token per user
$EDITOR ./hub.toml                       # paste the [[accounts]] block
cloudcode-hub --config ./hub.toml        # foreground; logs to stdout
# or
cloudcode-hub daemon start --config ./hub.toml   # background
```

### Agent (one-time setup)

**Prerequisites on the agent host:**
- `tmux` installed (`brew install tmux` / `apt install tmux`)
- `claude` installed and `claude /login` done as the same OS user that will run the agent

The agent itself never reads OAuth credentials; it just runs `tmux ... claude` under the same user, and claude finds its own credentials (keychain on macOS, `~/.claude/.credentials.json` on Linux).

```bash
cloudcode-agent --init                   # writes ./agent.toml template
$EDITOR ./agent.toml                     # paste [auth].registration_token
                                         # (the token your hub admin printed)
                                         # and set [hub].url

cloudcode-agent --config ./agent.toml    # foreground; logs to stdout
# or
cloudcode-agent daemon start --config ./agent.toml   # background
```

There's a **single global agent registration token**: every agent in the fleet uses the same one, and agent names are auto-generated (`<hostname>-<user>`) — there's no pre-registration list on the hub.

### Client (developer)

```toml
# ~/.config/cloudcode/config.toml
hub_url = "https://your-hub-host"
token   = "cc_xxx_from_admin"
```

Run `cloudcode`:

```bash
cloudcode                            # menu picks last agent + last workspace
cloudcode --agent peter-mbp          # pin a specific agent
```

You get a small **TUI picker** — first pick an agent, then a workspace, then drop into claude.

```
┌─ Select agent ────────────────┐    ┌─ Select workspace on alpha ────┐
│ ▶ alpha                       │    │   default                       │
│   beta                        │    │ ▶ proja                         │
│                               │    │   projb                         │
└───────────────────────────────┘    └─────────────────────────────────┘
 ↑↓ move · Enter pick · Esc/q quit   ↑↓ Enter · c create · d delete · Esc back · q quit
```

- **Arrow keys** (or `j` / `k`) to move; **Enter** to pick.
- `c` opens a small input box for a new workspace name.
- `d` asks `y/n` to delete the highlighted workspace.
- `Esc` on the workspace picker goes back to the agent picker; `Esc` (or `q`) on the agent picker quits cloudcode.

Pick a workspace and your terminal becomes the **native claude TUI** (status bar, todo board, diffs, permission prompts, claude's own `/clear` / `/login` / …). When claude exits (`/exit`, the process dies, etc) you're dropped right back at the workspace picker. From there `q` quits cloudcode.

Workspaces are named directories under `<workspace_root>/<account>/` on the agent host (default `~/cloudcode-agent/workspaces/<account>/<workspace>/`). Each workspace maps 1:1 to a tmux session named `cloudcode-<account>-<workspace>`. A workspace can be held by **at most one cloudcode session at a time per account** — the hub enforces this. Closing cloudcode does **not** kill the tmux session; long-running claude tasks (background fixes, agentic loops) keep going, and reopening the same workspace re-attaches to the running claude.

State persisted in `$XDG_STATE_HOME/cloudcode/`:
- `last_agent` — the most recent agent name
- `last_workspace/<agent>.txt` — most recent workspace per agent (used as the picker default)

#### Recording

Every session is recorded to an asciinema cast file on the agent at `~/.local/state/cloudcode/agent/recordings/<account>/<workspace>/<session_id>.cast`. Replay with `asciinema play <file>` for audit / debugging. Output only; keystrokes are not recorded (avoids leaking pasted tokens).

> Daemon-mode logs: `~/.local/state/cloudcode/{hub,agent}.log`. Lifecycle: `cloudcode-{hub,agent} daemon {status,stop,restart}`.

## Configuration reference

[`hub.example.toml`](hub.example.toml) · [`agent.example.toml`](agent.example.toml)

## Workspace sandbox (experimental)

Setting `[sandbox] enabled = true` in `agent.toml` wraps every `claude` (and its tmux session) in a per-workspace OS sandbox. On macOS this is Seatbelt — the same kernel-enforced isolation behind Apple's app containment — with a profile that:

- allows broad reads across the system so tooling (node, git, brew binaries, mise / asdf, …) keeps working;
- restricts writes to the active workspace, `~/.claude`, and a small set of cache / scratch dirs;
- isolates the workspace from neighbouring workspaces and other accounts — a session can only read its own workspace under `<workspace_root>`, everything else under that root is denied;
- denies persistence vectors a careless or compromised AI run might reach for (`~/.bashrc`, `.git/hooks/`, `~/Library/LaunchAgents`) and secrets (`~/.ssh`, `~/.gnupg`, `~/.aws`, Keychain) even where the surrounding path would otherwise be writable;
- leaves the network open so claude can reach the Anthropic API / package registries / git remotes.

Linux support is on the roadmap.

Off by default — opt in once you've confirmed the profile fits your projects' tooling.

## Acknowledgements

The macOS workspace sandbox design was inspired by [boxsh](https://github.com/xicilion/boxsh)'s approach to running AI coding agents inside OS-enforced isolation. The cloudcode implementation in `crates/agent/src/sandbox/` is independently authored — no boxsh code is used here. boxsh is GPL v3.

## License

MIT. The software is provided "as is", without warranty of any kind. The authors are not liable for any use that violates third-party terms of service.
