// Tab type + helpers for the workbench multi-session model.

import type { Terminal } from '@xterm/xterm';
import type { FitAddon } from '@xterm/addon-fit';
import type { WireSocket } from './wire';

export type TabStatus = 'connecting' | 'opening' | 'live' | 'closed' | 'error';

export type Tab = {
  id: string;
  agent: string;
  workspace: string;
  /** Tool used when opening the session. Effectively 'claude' as of
   *  v1.13; kept on the wire / Tab shape for back-compat. */
  tool?: string;
  status: TabStatus;
  errorMsg?: string;
  ws: WireSocket;
  term: Terminal;
  fitAddon: FitAddon;
  /** Has term.open() been called for this tab yet? Mutated by the
   * container-attach ref callback so we don't re-attach on every
   * render and infinite-loop. */
  opened: boolean;
};

/** Stable key used to deduplicate tabs. */
export function tabKey(agent: string, workspace: string): string {
  return `${agent}::${workspace}`;
}

/** Human-readable label shown in the tab bar. */
export function tabLabel(tab: Pick<Tab, 'agent' | 'workspace' | 'tool'>): string {
  if (tab.tool) {
    return `${tab.agent}·${tab.workspace}·${tab.tool}`;
  }
  return `${tab.agent}·${tab.workspace}`;
}
