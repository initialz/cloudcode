// WorkspaceTree: v1.13 workspace-first sidebar tree.
//
// Top level: per-account workspace list. Each row shows the workspace name
// and a lock-status badge. Clicking a row reveals an inline agent picker.
//
// Lock badge cases:
//   free            → no badge (grey dash in title attr)
//   locked by me    → green "current" badge (current tab's agent holds it)
//   locked by other → yellow "当前 agent: <name>" badge

import { useEffect, useState } from 'react';
import type { AgentItem, WorkspaceItem } from '@/lib/wire';
import { tabKey } from '@/lib/tabs';

type Props = {
  workspaces: WorkspaceItem[];
  loading: boolean;
  agents: AgentItem[];
  /** "agent::workspace" keys that already have an open tab. */
  openTabKeys: Set<string>;
  /** Key of the workspace tab currently in focus. */
  activeTabKey: string | null;
  onOpenWorkspace: (workspace: string, agent: string) => void;
  onResetWorkspace: (workspace: string) => void;
  onDeleteWorkspace: (workspace: string) => void;
  onRefreshAgents: () => void;
};

export default function WorkspaceTree({
  workspaces,
  loading,
  agents,
  openTabKeys,
  activeTabKey,
  onOpenWorkspace,
  onResetWorkspace,
  onDeleteWorkspace,
  onRefreshAgents,
}: Props) {
  // Which workspace row is currently expanded (showing the agent picker)
  const [expanded, setExpanded] = useState<string | null>(null);

  // Close expanded picker on Escape
  useEffect(() => {
    if (!expanded) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setExpanded(null);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [expanded]);

  function toggleExpand(name: string) {
    if (expanded === name) {
      setExpanded(null);
    } else {
      // Refresh agent list each time so it's current
      onRefreshAgents();
      setExpanded(name);
    }
  }

  if (loading) {
    return (
      <div className="px-3 py-2 text-xs text-zinc-400 dark:text-zinc-500">
        Loading workspaces...
      </div>
    );
  }

  if (workspaces.length === 0) {
    return (
      <div className="px-3 py-2 text-xs text-zinc-400 dark:text-zinc-500 italic">
        No workspaces yet. Create one above.
      </div>
    );
  }

  return (
    <div className="flex-1 overflow-y-auto">
      {workspaces.map((ws) => {
        const isExpanded = expanded === ws.name;
        // Determine if any tab is open for this workspace (any agent)
        const isLive = agents.some((a) => openTabKeys.has(tabKey(a.name, ws.name)));
        // Which agent key is currently active for this workspace
        const isActive = agents.some((a) => activeTabKey === tabKey(a.name, ws.name));

        return (
          <div key={ws.name}>
            <WorkspaceRow
              workspace={ws}
              isLive={isLive}
              isActive={isActive}
              isExpanded={isExpanded}
              onToggle={() => toggleExpand(ws.name)}
              onReset={() => onResetWorkspace(ws.name)}
              onDelete={() => onDeleteWorkspace(ws.name)}
            />

            {/* Inline agent picker */}
            {isExpanded && (
              <AgentPicker
                agents={agents}
                workspace={ws.name}
                openTabKeys={openTabKeys}
                activeTabKey={activeTabKey}
                onOpen={(agent) => {
                  onOpenWorkspace(ws.name, agent);
                  setExpanded(null);
                }}
              />
            )}
          </div>
        );
      })}
    </div>
  );
}

// ── WorkspaceRow ─────────────────────────────────────────────────────────────

function LockBadge({
  workspace,
  activeAgentForTab,
}: {
  workspace: WorkspaceItem;
  /** The agent that has the tab open in THIS browser session, if any. */
  activeAgentForTab: string | null;
}) {
  const { locked_by_agent } = workspace;

  if (!locked_by_agent) {
    // Free workspace
    return null;
  }

  if (locked_by_agent === activeAgentForTab) {
    // Locked by the agent whose tab is open in this session
    return (
      <span
        className="ml-1.5 px-1 py-0.5 rounded text-[10px] font-mono bg-emerald-100 dark:bg-emerald-900/40 text-emerald-700 dark:text-emerald-300 shrink-0"
        title={`Locked by ${locked_by_agent}`}
      >
        current
      </span>
    );
  }

  // Locked by a different agent
  return (
    <span
      className="ml-1.5 px-1 py-0.5 rounded text-[10px] font-mono bg-amber-100 dark:bg-amber-900/40 text-amber-700 dark:text-amber-300 shrink-0"
      title={`Locked by ${locked_by_agent}`}
    >
      当前 agent: {locked_by_agent}
    </span>
  );
}

function WorkspaceRow({
  workspace,
  isLive,
  isActive,
  isExpanded,
  onToggle,
  onReset,
  onDelete,
}: {
  workspace: WorkspaceItem;
  isLive: boolean;
  isActive: boolean;
  isExpanded: boolean;
  onToggle: () => void;
  onReset: () => void;
  onDelete: () => void;
}) {
  // For the "current" badge we need to know the agent that currently holds
  // the tab in this session. We do this by passing the lock holder to
  // LockBadge as the "activeAgentForTab" — since the tab was opened with
  // a specific agent, that agent must match the lock holder if it IS current.
  // When isLive is true and locked_by_agent is set, that's the "current" case.
  const activeAgentForTab = isLive ? workspace.locked_by_agent : null;

  return (
    <div
      className={`group flex items-center gap-1 px-2 py-1 text-xs font-mono cursor-pointer transition-colors ${
        isActive
          ? 'bg-zinc-200 dark:bg-zinc-700 text-zinc-900 dark:text-zinc-100'
          : isExpanded
            ? 'bg-zinc-100 dark:bg-zinc-800 text-zinc-800 dark:text-zinc-200'
            : 'text-zinc-600 dark:text-zinc-400 hover:bg-zinc-100 dark:hover:bg-zinc-800 hover:text-zinc-900 dark:hover:text-zinc-100'
      }`}
      onClick={onToggle}
    >
      {/* Live indicator dot */}
      <span className="shrink-0 w-2 text-center">
        {isLive ? (
          <span className="text-emerald-500 font-bold text-[10px]">●</span>
        ) : (
          <span className="text-transparent select-none text-[10px]" aria-hidden>·</span>
        )}
      </span>

      {/* Workspace name */}
      <span className="flex-1 truncate">{workspace.name}</span>

      {/* Lock badge */}
      <LockBadge workspace={workspace} activeAgentForTab={activeAgentForTab} />

      {/* Expand chevron */}
      <span
        className={`shrink-0 transition-transform duration-150 ${isExpanded ? 'rotate-90' : ''}`}
        aria-hidden
      >
        <svg width="8" height="8" viewBox="0 0 10 10" fill="none">
          <path d="M3 2L7 5L3 8" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </span>

      {/* Hover action buttons */}
      <span className="shrink-0 flex gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
        <button
          onClick={(e) => {
            e.stopPropagation();
            onReset();
          }}
          className="p-0.5 rounded text-zinc-400 hover:text-zinc-700 dark:hover:text-zinc-200 hover:bg-zinc-200 dark:hover:bg-zinc-700 transition-colors"
          title={`Reset ${workspace.name}`}
          aria-label={`Reset workspace ${workspace.name}`}
        >
          <ResetIcon />
        </button>
        <button
          onClick={(e) => {
            e.stopPropagation();
            onDelete();
          }}
          className="p-0.5 rounded text-zinc-400 hover:text-red-600 dark:hover:text-red-400 hover:bg-red-50 dark:hover:bg-red-950/40 transition-colors"
          title={`Delete ${workspace.name}`}
          aria-label={`Delete workspace ${workspace.name}`}
        >
          <TrashIcon />
        </button>
      </span>
    </div>
  );
}

// ── AgentPicker ──────────────────────────────────────────────────────────────

function AgentPicker({
  agents,
  workspace,
  openTabKeys,
  activeTabKey,
  onOpen,
}: {
  agents: AgentItem[];
  workspace: string;
  openTabKeys: Set<string>;
  activeTabKey: string | null;
  onOpen: (agent: string) => void;
}) {
  if (agents.length === 0) {
    return (
      <div className="pl-6 pr-2 py-1 text-xs text-zinc-400 dark:text-zinc-500 italic">
        No agents online
      </div>
    );
  }

  return (
    <div className="border-l-2 border-zinc-200 dark:border-zinc-700 ml-3 my-0.5">
      <div className="px-2 py-0.5 text-[10px] uppercase tracking-wide text-zinc-400 dark:text-zinc-500 font-sans">
        Open with agent
      </div>
      {agents.map((agent) => {
        const key = tabKey(agent.name, workspace);
        const isOpen = openTabKeys.has(key);
        const isActive = activeTabKey === key;
        return (
          <button
            key={agent.name}
            type="button"
            onClick={() => onOpen(agent.name)}
            className={`w-full flex items-center gap-1.5 pl-3 pr-2 py-0.5 text-xs font-mono text-left transition-colors ${
              isActive
                ? 'bg-zinc-200 dark:bg-zinc-700 text-zinc-900 dark:text-zinc-100'
                : 'text-zinc-600 dark:text-zinc-400 hover:bg-zinc-100 dark:hover:bg-zinc-800 hover:text-zinc-900 dark:hover:text-zinc-100'
            }`}
          >
            {/* Open tab indicator */}
            <span className="shrink-0 w-2">
              {isOpen ? (
                <span className="text-emerald-500 font-bold text-[10px]">●</span>
              ) : (
                <span className="text-transparent select-none text-[10px]" aria-hidden>·</span>
              )}
            </span>
            <span className="truncate">{agent.name}</span>
          </button>
        );
      })}
    </div>
  );
}

// ── Icons ────────────────────────────────────────────────────────────────────

function ResetIcon() {
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
      <path d="M3 3v5h5" />
    </svg>
  );
}

function TrashIcon() {
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M3 6h18" />
      <path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
      <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6" />
      <path d="M10 11v6" />
      <path d="M14 11v6" />
    </svg>
  );
}
