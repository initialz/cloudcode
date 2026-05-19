// Left sidebar: workspace tree (per-account) + account footer.
// v1.13: workspaces are global per account; each shows a lock-status badge.
// Clicking a workspace expands an inline agent picker.

import { useState } from 'react';
import Logo from '@/components/Logo';
import ConfirmDialog from '@/components/ConfirmDialog';
import type { AgentItem, WorkspaceItem } from '@/lib/wire';
import WorkspaceTree from '@/components/WorkspaceTree';

type Props = {
  account: string;
  workspaces: WorkspaceItem[];
  workspacesLoading: boolean;
  agents: AgentItem[];
  openTabKeys: Set<string>;
  activeTabKey: string | null;
  onOpenWorkspace: (workspace: string, agent: string) => void;
  onCreateWorkspace: (name: string) => void;
  onResetWorkspace: (workspace: string) => void;
  onDeleteWorkspace: (workspace: string) => void;
  onRefreshWorkspaces: () => void;
  onRefreshAgents: () => void;
  onSettings: () => void;
  onLogout: () => void;
};

type ConfirmState = {
  title: string;
  body: string;
  label: string;
  danger: boolean;
  onConfirm: () => void;
};

export default function Sidebar({
  account,
  workspaces,
  workspacesLoading,
  agents,
  openTabKeys,
  activeTabKey,
  onOpenWorkspace,
  onCreateWorkspace,
  onResetWorkspace,
  onDeleteWorkspace,
  onRefreshWorkspaces,
  onRefreshAgents,
  onSettings,
  onLogout,
}: Props) {
  const [showCreate, setShowCreate] = useState(false);
  const [createName, setCreateName] = useState('');
  const [confirm, setConfirm] = useState<ConfirmState | null>(null);

  function submitCreate() {
    const name = createName.trim();
    if (!name) return;
    onCreateWorkspace(name);
    setShowCreate(false);
    setCreateName('');
  }

  function askReset(workspace: string) {
    setConfirm({
      title: 'Reset workspace?',
      body: `This will reset "${workspace}" to a fresh state.`,
      label: 'Reset',
      danger: false,
      onConfirm: () => {
        setConfirm(null);
        onResetWorkspace(workspace);
      },
    });
  }

  function askDelete(workspace: string) {
    setConfirm({
      title: 'Delete workspace?',
      body: `This will permanently delete "${workspace}".`,
      label: 'Delete',
      danger: true,
      onConfirm: () => {
        setConfirm(null);
        onDeleteWorkspace(workspace);
      },
    });
  }

  return (
    <>
      <aside className="flex flex-col w-64 shrink-0 border-r border-zinc-200 dark:border-zinc-800 bg-zinc-50 dark:bg-zinc-900 h-full overflow-hidden">
        {/* Header */}
        <div className="flex items-center gap-2 px-3 py-2.5 border-b border-zinc-200 dark:border-zinc-800 shrink-0">
          <Logo size={18} className="text-zinc-700 dark:text-zinc-300 shrink-0" />
          <span className="text-sm font-semibold text-zinc-800 dark:text-zinc-200 select-none">
            Workbench
          </span>
        </div>

        {/* + New workspace button */}
        <div className="px-3 py-2 border-b border-zinc-200 dark:border-zinc-800 shrink-0">
          <button
            onClick={() => {
              setCreateName('');
              setShowCreate(true);
            }}
            className="w-full flex items-center justify-center gap-1.5 px-2 py-1.5 rounded-md text-xs font-mono text-zinc-500 dark:text-zinc-400 border border-dashed border-zinc-300 dark:border-zinc-700 hover:border-zinc-400 dark:hover:border-zinc-500 hover:text-zinc-700 dark:hover:text-zinc-200 transition-colors"
          >
            <svg width="10" height="10" viewBox="0 0 10 10" fill="none" aria-hidden>
              <path d="M5 1v8M1 5h8" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
            </svg>
            New workspace
          </button>
        </div>

        {/* Tree */}
        <div className="flex-1 overflow-y-auto py-1">
          <WorkspaceTree
            workspaces={workspaces}
            loading={workspacesLoading}
            agents={agents}
            openTabKeys={openTabKeys}
            activeTabKey={activeTabKey}
            onOpenWorkspace={onOpenWorkspace}
            onResetWorkspace={askReset}
            onDeleteWorkspace={askDelete}
            onRefreshAgents={onRefreshAgents}
          />
        </div>

        {/* Refresh + Account / actions footer */}
        <div className="shrink-0 px-3 py-2.5 border-t border-zinc-200 dark:border-zinc-800">
          <div className="text-xs text-zinc-600 dark:text-zinc-400 font-mono truncate mb-1.5">
            {account}
          </div>
          <div className="flex gap-2">
            <button
              onClick={onRefreshWorkspaces}
              className="text-xs text-zinc-400 dark:text-zinc-500 hover:text-zinc-700 dark:hover:text-zinc-300 transition-colors"
            >
              refresh
            </button>
            <span className="text-zinc-300 dark:text-zinc-700 select-none">·</span>
            <button
              onClick={onSettings}
              className="text-xs text-zinc-400 dark:text-zinc-500 hover:text-zinc-700 dark:hover:text-zinc-300 transition-colors"
            >
              settings
            </button>
            <span className="text-zinc-300 dark:text-zinc-700 select-none">·</span>
            <button
              onClick={onLogout}
              className="text-xs text-zinc-400 dark:text-zinc-500 hover:text-red-600 dark:hover:text-red-400 transition-colors"
            >
              logout
            </button>
          </div>
        </div>
      </aside>

      {/* Create workspace modal */}
      {showCreate && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
          <div className="bg-white dark:bg-zinc-900 border border-zinc-200 dark:border-zinc-800 rounded-xl shadow-xl p-6 w-full max-w-sm mx-4">
            <h3 className="text-base font-semibold text-zinc-900 dark:text-zinc-100 mb-4">
              New workspace
            </h3>

            <div className="mb-4">
              <label className="block text-xs font-medium text-zinc-500 dark:text-zinc-400 mb-1 uppercase tracking-wide">
                Name
              </label>
              <input
                type="text"
                placeholder="workspace name"
                value={createName}
                onChange={(e) => setCreateName(e.target.value)}
                onKeyDown={(e) => e.key === 'Enter' && submitCreate()}
                autoFocus
                className="w-full rounded-lg border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-800 px-3 py-2 text-sm text-zinc-900 dark:text-zinc-100 placeholder-zinc-400 focus:outline-none focus:ring-2 focus:ring-zinc-500 font-mono"
              />
            </div>

            <div className="flex gap-2 justify-end">
              <button
                onClick={() => setShowCreate(false)}
                className="text-sm px-3 py-1.5 rounded-lg border border-zinc-200 dark:border-zinc-700 text-zinc-600 dark:text-zinc-400 hover:bg-zinc-50 dark:hover:bg-zinc-800 transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={submitCreate}
                disabled={!createName.trim()}
                className="text-sm px-3 py-1.5 rounded-lg bg-zinc-900 dark:bg-zinc-100 text-white dark:text-zinc-900 hover:bg-zinc-700 dark:hover:bg-zinc-300 disabled:opacity-50 transition-colors"
              >
                Create
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Confirm dialog */}
      {confirm && (
        <ConfirmDialog
          title={confirm.title}
          body={confirm.body}
          confirmLabel={confirm.label}
          danger={confirm.danger}
          onConfirm={confirm.onConfirm}
          onCancel={() => setConfirm(null)}
        />
      )}
    </>
  );
}
