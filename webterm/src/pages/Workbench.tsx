// IDE-style workbench: left sidebar (workspace tree) + right tab bar + xterm area.
// Owns:
//   1. Control WS — menu phase (list workspaces / agents, create/delete/reset)
//   2. Per-tab PTY WS — one independent WireSocket + Terminal per open workspace

import {
  useEffect,
  useRef,
  useState,
  useCallback,
  useReducer,
} from 'react';
import { useNavigate } from 'react-router-dom';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import '@xterm/xterm/css/xterm.css';

import { apiClient } from '@/lib/api';
import {
  WireSocket,
  type AgentItem,
  type WorkspaceItem,
  type HubMsg,
} from '@/lib/wire';
import { effectiveTheme, getStoredTheme, type Theme } from '@/lib/theme';
import { type Tab, tabKey } from '@/lib/tabs';
import {
  DEFAULT_PREFERENCES,
  parsePreferences,
  serializePreferences,
  type Preferences,
} from '@/lib/preferences';
import Sidebar from '@/components/Sidebar';
import TabBar from '@/components/TabBar';
import SettingsDialog from '@/components/SettingsDialog';
import ConfirmDialog from '@/components/ConfirmDialog';

// ── xterm theme helpers ──────────────────────────────────────────────────────

function darkXterm() {
  return { background: '#18181b', foreground: '#fafafa', cursor: '#fafafa' };
}

function lightXterm() {
  return { background: '#ffffff', foreground: '#18181b', cursor: '#18181b' };
}

function xtermTheme(dark: boolean) {
  return dark ? darkXterm() : lightXterm();
}

// ── Tab state reducer ────────────────────────────────────────────────────────

type TabAction =
  | { type: 'ADD'; tab: Tab }
  | { type: 'UPDATE'; id: string; patch: Partial<Tab> }
  | { type: 'REMOVE'; id: string };

function tabsReducer(state: Tab[], action: TabAction): Tab[] {
  switch (action.type) {
    case 'ADD':
      return [...state, action.tab];
    case 'UPDATE':
      return state.map((t) => (t.id === action.id ? { ...t, ...action.patch } : t));
    case 'REMOVE':
      return state.filter((t) => t.id !== action.id);
    default:
      return state;
  }
}

// ── Takeover confirm state ────────────────────────────────────────────────────

type TakeoverPending = {
  workspace: string;
  agent: string;
  lockedBy: string;
};

// ── Workbench ────────────────────────────────────────────────────────────────

export default function Workbench() {
  const navigate = useNavigate();

  // Auth
  const [account, setAccount] = useState('');
  const [authLoading, setAuthLoading] = useState(true);

  // Control WS (menu phase)
  const ctrlWsRef = useRef<WireSocket | null>(null);
  const [ctrlReady, setCtrlReady] = useState(false);

  // Per-account workspace list + agent list
  const [workspaces, setWorkspaces] = useState<WorkspaceItem[]>([]);
  const [workspacesLoading, setWorkspacesLoading] = useState(true);
  const [agents, setAgents] = useState<AgentItem[]>([]);

  // Tabs
  const [tabs, dispatchTabs] = useReducer(tabsReducer, []);
  const tabsRef = useRef<Tab[]>(tabs);
  tabsRef.current = tabs;
  const [activeTabId, setActiveTabId] = useState<string | null>(null);

  // Settings dialog
  const [showSettings, setShowSettings] = useState(false);

  // Takeover confirm dialog (locked workspace)
  const [takeoverPending, setTakeoverPending] = useState<TakeoverPending | null>(null);

  // Per-user preferences (default args, future things). Loaded from the hub on
  // mount; kept in a ref so non-reactive callbacks see fresh values without
  // re-binding.
  const [preferences, setPreferences] = useState<Preferences>(DEFAULT_PREFERENCES);
  const preferencesRef = useRef<Preferences>(preferences);
  preferencesRef.current = preferences;

  // Transient error toasts (non-fatal hub events).
  type Toast = { id: string; message: string };
  const [toasts, setToasts] = useState<Toast[]>([]);
  const addToast = useCallback((message: string) => {
    const id = crypto.randomUUID();
    setToasts((prev) => [...prev, { id, message }]);
    setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id));
    }, 6000);
  }, []);
  const dismissToast = useCallback((id: string) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  // Refresh timer ref (30s poll)
  const pollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Container DOM nodes per tab id.
  const containersRef = useRef<Map<string, HTMLDivElement>>(new Map());

  // Per-tab copy-mode flag (see attachContainer).
  const copyModeRef = useRef<Map<string, boolean>>(new Map());

  // ── Auth check ─────────────────────────────────────────────────────────────

  useEffect(() => {
    apiClient
      .me()
      .then((me) => {
        setAccount(me.account);
        setAuthLoading(false);
      })
      .catch(() => {
        navigate('/login', { replace: true });
      });
  }, [navigate]);

  // ── Preferences load ─────────────────────────────────────────────────────

  useEffect(() => {
    if (authLoading) return;
    apiClient
      .getPreferences()
      .then((resp) => setPreferences(parsePreferences(resp.preferences)))
      .catch(() => {});
  }, [authLoading]);

  const savePreferences = useCallback(async (next: Preferences) => {
    setPreferences(next);
    try {
      await apiClient.putPreferences(serializePreferences(next));
    } catch {
      addToast('Could not save preferences — your change applies to this tab but did not persist.');
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // ── Control WS helpers ─────────────────────────────────────────────────────

  const refreshWorkspaces = useCallback(() => {
    if (!ctrlWsRef.current?.connected) return;
    ctrlWsRef.current.send({ type: 'list_workspaces' });
  }, []);

  const refreshAgents = useCallback(() => {
    if (!ctrlWsRef.current?.connected) return;
    ctrlWsRef.current.send({ type: 'list_agents' });
  }, []);

  const schedulePoll = useCallback(() => {
    if (pollTimerRef.current) clearTimeout(pollTimerRef.current);
    pollTimerRef.current = setTimeout(() => {
      refreshWorkspaces();
      schedulePoll();
    }, 30_000);
  }, [refreshWorkspaces]);

  // ── Control WS message handler ─────────────────────────────────────────────

  const handleCtrlMsg = useCallback(
    (msg: HubMsg) => {
      switch (msg.type) {
        case 'welcome':
          setCtrlReady(true);
          // Fetch both workspace list and agent list on connect
          ctrlWsRef.current?.send({ type: 'list_workspaces' });
          ctrlWsRef.current?.send({ type: 'list_agents' });
          break;

        case 'workspace_list':
          setWorkspaces(msg.items);
          setWorkspacesLoading(false);
          break;

        case 'agent_list':
          setAgents(msg.items);
          break;

        case 'workspace_created':
        case 'workspace_deleted':
        case 'workspace_reset':
          refreshWorkspaces();
          break;

        case 'rejected':
          navigate('/login', { replace: true });
          break;

        default:
          break;
      }
    },
    [navigate, refreshWorkspaces],
  );

  // ── Build control WS ───────────────────────────────────────────────────────

  useEffect(() => {
    if (authLoading) return;

    const ws = new WireSocket({
      onMessage: handleCtrlMsg,
      onBinary: () => {},
      onClose: () => {
        setCtrlReady(false);
      },
      onError: () => {
        setCtrlReady(false);
      },
    });

    ws.connect();
    ctrlWsRef.current = ws;
    schedulePoll();

    return () => {
      if (pollTimerRef.current) clearTimeout(pollTimerRef.current);
      ws.close();
      ctrlWsRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [authLoading]);

  // ── Sidebar callbacks ──────────────────────────────────────────────────────

  function handleCreateWorkspace(name: string) {
    if (!ctrlWsRef.current?.connected) return;
    ctrlWsRef.current.send({ type: 'create_workspace', name });
  }

  function handleResetWorkspace(workspace: string) {
    if (!ctrlWsRef.current?.connected) return;
    // Find any open tab for this workspace (any agent)
    const openTab = tabsRef.current.find((t) => t.workspace === workspace);
    const doReset = () => {
      ctrlWsRef.current?.send({ type: 'reset_workspace', name: workspace });
    };
    if (openTab) {
      closeTabRef.current(openTab.id);
      setTimeout(doReset, 400);
    } else {
      doReset();
    }
  }

  function handleDeleteWorkspace(workspace: string) {
    if (!ctrlWsRef.current?.connected) return;
    const openTab = tabsRef.current.find((t) => t.workspace === workspace);
    const doDelete = () => {
      ctrlWsRef.current?.send({ type: 'delete_workspace', name: workspace });
    };
    if (openTab) {
      closeTabRef.current(openTab.id);
      setTimeout(doDelete, 400);
    } else {
      doDelete();
    }
  }

  // ── Open tab ──────────────────────────────────────────────────────────────

  const openTab = useCallback(
    (workspace: string, agent: string, force?: boolean) => {
      // Check if workspace is locked by a different agent; surface takeover
      // dialog unless force is already set (called from confirm handler).
      if (!force) {
        const ws = workspaces.find((w) => w.name === workspace);
        if (ws?.locked_by_agent && ws.locked_by_agent !== agent) {
          setTakeoverPending({ workspace, agent, lockedBy: ws.locked_by_agent });
          return;
        }
      }

      // Deduplicate by agent::workspace
      const key = tabKey(agent, workspace);
      const existing = tabsRef.current.find(
        (t) => tabKey(t.agent, t.workspace) === key,
      );
      if (existing) {
        setActiveTabId(existing.id);
        return;
      }

      const isDark = effectiveTheme(getStoredTheme()) === 'dark';
      const term = new Terminal({
        cursorBlink: true,
        scrollback: 10000,
        fontFamily: 'ui-monospace, Menlo, Monaco, monospace',
        fontSize: 14,
        theme: xtermTheme(isDark),
      });
      const fitAddon = new FitAddon();
      const linksAddon = new WebLinksAddon();
      term.loadAddon(fitAddon);
      term.loadAddon(linksAddon);

      // OSC 52 clipboard write.
      term.parser.registerOscHandler(52, (data) => {
        const sep = data.indexOf(';');
        if (sep < 0) return false;
        const targets = data.substring(0, sep);
        const payload = data.substring(sep + 1);
        if (targets !== '' && !targets.includes('c')) return false;
        let text: string;
        try {
          const binary = atob(payload);
          const bytes = new Uint8Array(binary.length);
          for (let i = 0; i < binary.length; i++) {
            bytes[i] = binary.charCodeAt(i);
          }
          text = new TextDecoder('utf-8').decode(bytes);
        } catch {
          return false;
        }
        navigator.clipboard.writeText(text).catch(() => {});
        return true;
      });

      const id = crypto.randomUUID();

      const ws = new WireSocket({
        onMessage: (msg) => handleTabMsg(id, workspace, agent, force ?? false, msg),
        onBinary: (data) => {
          const tab = tabsRef.current.find((t) => t.id === id);
          tab?.term.write(data);
        },
        onClose: () => {
          refreshWorkspaces();
          closeTabRef.current(id);
        },
        onError: () => {
          closeTabRef.current(id);
        },
      });

      // Wire terminal input → WS
      term.onData((data) => {
        const tab = tabsRef.current.find((t) => t.id === id);
        if (tab?.ws.connected) {
          tab.ws.sendBinary(new TextEncoder().encode(data));
        }
      });

      const newTab: Tab = {
        id,
        workspace,
        agent,
        status: 'connecting',
        ws,
        term,
        fitAddon,
        opened: false,
      };

      // Cmd+C bridge for tmux copy-mode.
      term.attachCustomKeyEventHandler((ev) => {
        if (ev.type !== 'keydown') return true;
        const isCmdC =
          ev.metaKey &&
          !ev.ctrlKey &&
          !ev.shiftKey &&
          !ev.altKey &&
          ev.key.toLowerCase() === 'c';
        if (!isCmdC) return true;
        if (copyModeRef.current.get(id) !== true) return true;
        const me = tabsRef.current.find((t) => t.id === id);
        if (me?.ws.connected) {
          me.ws.sendBinary(new TextEncoder().encode('y'));
        }
        copyModeRef.current.set(id, false);
        return false;
      });

      dispatchTabs({ type: 'ADD', tab: newTab });
      setActiveTabId(id);
      ws.connect();
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [workspaces],
  );

  // ── PTY WS message handler (per tab) ──────────────────────────────────────

  function handleTabMsg(
    tabId: string,
    workspace: string,
    agent: string,
    force: boolean,
    msg: HubMsg,
  ) {
    switch (msg.type) {
      case 'welcome': {
        dispatchTabs({ type: 'UPDATE', id: tabId, patch: { status: 'opening' } });
        // v1.13: send open_session directly with workspace + agent (no select_agent dance).
        const tab = tabsRef.current.find((t) => t.id === tabId);
        if (!tab) break;
        let cols = 80;
        let rows = 24;
        if (containersRef.current.has(tab.id)) {
          try {
            tab.fitAddon.fit();
            cols = tab.term.cols;
            rows = tab.term.rows;
          } catch {
            // container not yet measured
          }
        }
        const args = preferencesRef.current.toolArgs['claude'] ?? [];
        tab.ws.send({
          type: 'open_session',
          workspace,
          agent,
          ...(force ? { force: true } : {}),
          cols,
          rows,
          ...(args.length > 0 ? { claude_args: args } : {}),
        });
        break;
      }
      case 'session_opened': {
        dispatchTabs({ type: 'UPDATE', id: tabId, patch: { status: 'live' } });
        setTimeout(() => {
          const tab = tabsRef.current.find((t) => t.id === tabId);
          if (!tab || !containersRef.current.has(tab.id)) return;
          try {
            tab.fitAddon.fit();
            tab.ws.send({ type: 'resize', cols: tab.term.cols, rows: tab.term.rows });
          } catch {
            // ignore
          }
          if (tab.id === activeTabIdRef.current) {
            tab.term.focus();
          }
          refreshWorkspaces();
        }, 50);
        break;
      }
      case 'session_error':
        addToast(msg.message || 'Session error');
        refreshWorkspaces();
        break;
      case 'session_closed':
        refreshWorkspaces();
        closeTabRef.current(tabId);
        break;
      default:
        break;
    }
  }

  const activeTabIdRef = useRef<string | null>(null);
  activeTabIdRef.current = activeTabId;

  // ── Close tab ─────────────────────────────────────────────────────────────

  const closeTab = useCallback(
    (id: string) => {
      const all = tabsRef.current;
      const tab = all.find((t) => t.id === id);
      if (tab) {
        tab.ws.close();
        tab.term.dispose();
      }
      dispatchTabs({ type: 'REMOVE', id });

      setActiveTabId((prev) => {
        if (prev !== id) return prev;
        const remaining = all.filter((t) => t.id !== id);
        if (remaining.length === 0) return null;
        const idx = all.findIndex((t) => t.id === id);
        return remaining[Math.min(idx, remaining.length - 1)].id;
      });
    },
    [],
  );

  const closeTabRef = useRef<(id: string) => void>(() => {});
  closeTabRef.current = closeTab;

  // ── Switch active tab ─────────────────────────────────────────────────────

  const selectTab = useCallback((id: string) => {
    setActiveTabId(id);
    requestAnimationFrame(() => {
      const tab = tabsRef.current.find((t) => t.id === id);
      if (!tab || !containersRef.current.has(tab.id)) return;
      try {
        tab.fitAddon.fit();
        if (tab.ws.connected) {
          tab.ws.send({ type: 'resize', cols: tab.term.cols, rows: tab.term.rows });
        }
      } catch {
        // ignore
      }
      tab.term.focus();
    });
  }, []);

  // ── Container ref callbacks (attach xterm after DOM mount) ────────────────

  const attachContainer = useCallback(
    (tabId: string, el: HTMLDivElement | null) => {
      if (!el) {
        containersRef.current.delete(tabId);
        return;
      }
      containersRef.current.set(tabId, el);
      const tab = tabsRef.current.find((t) => t.id === tabId);
      if (!tab || tab.opened) return;
      try {
        tab.term.open(el);
        tab.fitAddon.fit();
        tab.opened = true;
      } catch {
        // StrictMode double-mount
      }

      // Track copy-mode state for Cmd+C bridge.
      let downX = 0;
      let downY = 0;
      let movedDuringDrag = false;
      el.addEventListener(
        'mousedown',
        (ev) => {
          downX = ev.clientX;
          downY = ev.clientY;
          movedDuringDrag = false;
          copyModeRef.current.set(tabId, false);
        },
        true,
      );
      el.addEventListener(
        'mousemove',
        (ev) => {
          if (!(ev.buttons & 1)) return;
          if (Math.abs(ev.clientX - downX) > 4 || Math.abs(ev.clientY - downY) > 4) {
            movedDuringDrag = true;
          }
        },
        true,
      );
      el.addEventListener(
        'mouseup',
        () => {
          if (movedDuringDrag) copyModeRef.current.set(tabId, true);
        },
        true,
      );
    },
    [],
  );

  // ── ResizeObserver: fit active terminal on resize ─────────────────────────

  useEffect(() => {
    if (!activeTabId) return;
    const tab = tabsRef.current.find((t) => t.id === activeTabId);
    const el = containersRef.current.get(activeTabId);
    if (!tab || !el) return;

    let timer: ReturnType<typeof setTimeout> | null = null;
    const ro = new ResizeObserver(() => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => {
        try {
          tab.fitAddon.fit();
          if (tab.ws.connected) {
            tab.ws.send({ type: 'resize', cols: tab.term.cols, rows: tab.term.rows });
          }
        } catch {
          // ignore
        }
      }, 150);
    });
    ro.observe(el);
    return () => {
      ro.disconnect();
      if (timer) clearTimeout(timer);
    };
  }, [activeTabId]);

  // ── Theme change: update all terminals ───────────────────────────────────

  function handleThemeChange(t: Theme) {
    const isDark = effectiveTheme(t) === 'dark';
    tabsRef.current.forEach((tab) => {
      tab.term.options.theme = xtermTheme(isDark);
    });
  }

  // ── Logout ────────────────────────────────────────────────────────────────

  function handleLogout() {
    apiClient.logout().finally(() => navigate('/login', { replace: true }));
  }

  // ── Computed: set of open tab keys ───────────────────────────────────────

  const openTabKeys = new Set(tabs.map((t) => tabKey(t.agent, t.workspace)));
  const activeTab = tabs.find((t) => t.id === activeTabId) ?? null;
  const activeTabKey = activeTab ? tabKey(activeTab.agent, activeTab.workspace) : null;

  // ── Render ────────────────────────────────────────────────────────────────

  if (authLoading) {
    return (
      <div className="h-full flex items-center justify-center text-zinc-500 text-sm">
        Loading...
      </div>
    );
  }

  void ctrlReady;

  return (
    <div className="h-full flex overflow-hidden bg-white dark:bg-zinc-950">
      {/* Left sidebar */}
      <Sidebar
        account={account}
        workspaces={workspaces}
        workspacesLoading={workspacesLoading}
        agents={agents}
        openTabKeys={openTabKeys}
        activeTabKey={activeTabKey}
        onOpenWorkspace={openTab}
        onCreateWorkspace={handleCreateWorkspace}
        onResetWorkspace={handleResetWorkspace}
        onDeleteWorkspace={handleDeleteWorkspace}
        onRefreshWorkspaces={refreshWorkspaces}
        onRefreshAgents={refreshAgents}
        onSettings={() => setShowSettings(true)}
        onLogout={handleLogout}
      />

      {/* Right: tab bar + terminal area */}
      <div className="flex-1 flex flex-col overflow-hidden">
        {/* Tab bar */}
        <TabBar
          tabs={tabs}
          activeTabId={activeTabId}
          onSelect={selectTab}
          onClose={closeTab}
        />

        {/* Terminal containers */}
        <div className="flex-1 relative overflow-hidden bg-white dark:bg-zinc-950">
          {tabs.length === 0 && (
            <div className="absolute inset-0 flex items-center justify-center text-sm text-zinc-400 dark:text-zinc-600 select-none">
              Open a workspace from the sidebar to start
            </div>
          )}

          {tabs.map((tab) => (
            <div
              key={tab.id}
              ref={(el) => attachContainer(tab.id, el)}
              className={`absolute inset-0 ${tab.id === activeTabId ? 'block' : 'hidden'}`}
            >
              {(tab.status === 'connecting' || tab.status === 'opening') && (
                <div className="absolute inset-0 flex items-center justify-center bg-white/80 dark:bg-zinc-950/80 z-10 pointer-events-none">
                  <span className="text-sm text-zinc-500 dark:text-zinc-400">
                    {tab.status === 'connecting' ? 'Connecting...' : 'Opening session...'}
                  </span>
                </div>
              )}
              {(tab.status === 'closed' || tab.status === 'error') && (
                <div className="absolute inset-0 flex flex-col items-center justify-center gap-4 z-10 bg-white/90 dark:bg-zinc-950/90">
                  <div className="rounded-lg bg-red-50 dark:bg-red-950 border border-red-200 dark:border-red-900 px-6 py-4 text-sm text-red-700 dark:text-red-400 max-w-md text-center">
                    {tab.errorMsg ?? 'Session ended'}
                  </div>
                  <button
                    onClick={() => closeTab(tab.id)}
                    className="text-sm px-4 py-2 rounded-lg border border-zinc-200 dark:border-zinc-700 text-zinc-600 dark:text-zinc-400 hover:bg-zinc-50 dark:hover:bg-zinc-800 transition-colors"
                  >
                    Close tab
                  </button>
                </div>
              )}
            </div>
          ))}
        </div>
      </div>

      {/* Settings modal */}
      {showSettings && (
        <SettingsDialog
          onClose={() => setShowSettings(false)}
          onThemeChange={handleThemeChange}
          preferences={preferences}
          onSavePreferences={savePreferences}
        />
      )}

      {/* Takeover confirm dialog */}
      {takeoverPending && (
        <ConfirmDialog
          title="Take over workspace?"
          body={`Workspace '${takeoverPending.workspace}' is currently held by agent '${takeoverPending.lockedBy}'. Take over? The other agent's local copy will be cleared on next online.`}
          confirmLabel="Take over"
          danger={false}
          onConfirm={() => {
            const { workspace, agent } = takeoverPending;
            setTakeoverPending(null);
            openTab(workspace, agent, true);
          }}
          onCancel={() => setTakeoverPending(null)}
        />
      )}

      {/* Transient error toasts */}
      {toasts.length > 0 && (
        <div className="pointer-events-none fixed bottom-4 right-4 z-50 flex max-w-md flex-col gap-2">
          {toasts.map((t) => (
            <div
              key={t.id}
              className="pointer-events-auto flex items-start gap-2 rounded-md border border-red-200 dark:border-red-900 bg-red-50 dark:bg-red-950 px-3 py-2 text-xs font-mono text-red-700 dark:text-red-300 shadow-lg"
              role="alert"
            >
              <span className="flex-1 break-words">{t.message}</span>
              <button
                type="button"
                onClick={() => dismissToast(t.id)}
                className="shrink-0 rounded p-0.5 opacity-60 hover:opacity-100 hover:bg-red-100 dark:hover:bg-red-900"
                aria-label="Dismiss"
              >
                <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
                  <path d="M2 2L8 8M8 2L2 8" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
                </svg>
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
