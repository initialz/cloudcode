// Tab type + helpers for the workbench multi-session model.

import type { Terminal } from '@xterm/xterm';
import type { FitAddon } from '@xterm/addon-fit';
import type { WireSocket } from './wire';

export type TabStatus = 'connecting' | 'opening' | 'live' | 'closed' | 'error';

export type Tab = {
  id: string;
  agent: string;
  workspace: string;
  status: TabStatus;
  errorMsg?: string;
  ws: WireSocket;
  term: Terminal;
  fitAddon: FitAddon;
  /** Set by the container div ref callback after first render. */
  container: HTMLDivElement | null;
};

/** Stable key used to deduplicate tabs. */
export function tabKey(agent: string, workspace: string): string {
  return `${agent}::${workspace}`;
}

/** Human-readable label shown in the tab bar. */
export function tabLabel(tab: Pick<Tab, 'agent' | 'workspace'>): string {
  return `${tab.agent}·${tab.workspace}`;
}
