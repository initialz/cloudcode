// Per-user webterm preferences. The hub stores these as an opaque JSON
// blob keyed by account; webterm owns the schema and validates on read.
//
// Today this is just default CLI args per tool. Extend the shape by
// adding more keys to `Preferences`; old rows without the new keys
// will fall back to defaults via the parse step.

import { KNOWN_TOOLS, type Tool } from './tools';

export type Preferences = {
  /** Default argv appended to each tool when starting a session.
   *  Each entry is an already-split argv element (so "--model" and
   *  "claude-3-opus" are two separate strings). */
  toolArgs: Record<Tool, string[]>;
};

function emptyToolArgs(): Record<Tool, string[]> {
  // Build with an explicit object so TS keeps the typed key shape; using
  // Object.fromEntries widens the keys back to `string` here.
  const out = {} as Record<Tool, string[]>;
  for (const t of KNOWN_TOOLS) out[t] = [];
  return out;
}

export const DEFAULT_PREFERENCES: Preferences = {
  toolArgs: emptyToolArgs(),
};

// ── Wire shape ──────────────────────────────────────────────────────────────
// Stored on the hub as { tool_args: { <tool>: [...] } }. As of v1.13
// the only entry that ever ends up populated is `claude`; the map shape
// is preserved so a future tool addition doesn't need a schema bump.
// Keep the wire keys snake_case to match the rest of the hub's JSON
// conventions; map to camelCase at the boundary.

type WireShape = {
  tool_args?: Record<string, string[]>;
};

/** Parse a server-side blob into a typed Preferences, filling in any
 *  missing pieces with defaults. Never throws — bad data falls back. */
export function parsePreferences(blob: unknown): Preferences {
  const base: Preferences = {
    toolArgs: { ...DEFAULT_PREFERENCES.toolArgs },
  };
  if (!blob || typeof blob !== 'object') return base;
  const wire = blob as WireShape;
  if (wire.tool_args && typeof wire.tool_args === 'object') {
    for (const tool of KNOWN_TOOLS) {
      const v = wire.tool_args[tool];
      if (Array.isArray(v) && v.every((x) => typeof x === 'string')) {
        base.toolArgs[tool] = v;
      }
    }
  }
  return base;
}

/** Reverse of parsePreferences — produce the wire blob for storage. */
export function serializePreferences(prefs: Preferences): WireShape {
  const out: Record<string, string[]> = {};
  for (const t of KNOWN_TOOLS) out[t] = prefs.toolArgs[t] ?? [];
  return { tool_args: out };
}

// ── Args ↔ text helpers ─────────────────────────────────────────────────────

/** Display args as a single string for an <input>. Naive: just space-join.
 *  Round-trips correctly as long as users don't put spaces inside args. */
export function argsToText(args: string[]): string {
  return args.join(' ');
}

/** Parse the text typed into the input back into argv. Whitespace-split,
 *  empty entries dropped. Doesn't handle quoted args — keep it dumb
 *  until someone hits a real need (claude flags rarely contain spaces). */
export function textToArgs(text: string): string[] {
  return text
    .trim()
    .split(/\s+/)
    .filter((s) => s.length > 0);
}
