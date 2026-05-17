// Tools the webterm UI offers in its open / split dropdowns. The
// agent's agent.toml [tools] map is the actual source of truth at
// runtime — anything not configured there will get a friendly error
// from the hub. Keep this list small and hand-edited for now; later
// we'll pull it dynamically from the agent's Hello frame.
export const KNOWN_TOOLS = ['claude', 'codex'] as const;
export type Tool = (typeof KNOWN_TOOLS)[number];

export const DEFAULT_TOOL: Tool = 'claude';
