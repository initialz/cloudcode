// Tools the webterm UI knows how to launch in a workspace. As of v1.13
// this is claude-only — other AI CLIs (formerly run side-by-side in a
// split pane) are now expected to be invoked from inside claude via
// plugins / MCP. The agent's agent.toml [tools] map is still the
// runtime source of truth: anything not configured there gets a
// friendly error from the hub.
export const KNOWN_TOOLS = ['claude'] as const;
export type Tool = (typeof KNOWN_TOOLS)[number];

export const DEFAULT_TOOL: Tool = 'claude';

/** Tools to show for a given agent. Pre-v1.13 agents don't report a
 *  list and end up with `undefined`/empty here — treat that as
 *  "unknown, fall back to KNOWN_TOOLS" so we don't blank out their
 *  open menu. Newer agents send exactly `["claude"]`. */
export function toolsForAgent(reported: readonly string[] | undefined): Tool[] {
  if (!reported || reported.length === 0) return [...KNOWN_TOOLS];
  return KNOWN_TOOLS.filter((t) => reported.includes(t));
}
