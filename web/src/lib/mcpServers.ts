// Client for the satchel-side multi-MCP config endpoints. Auth headers
// stay server-side; the browser only sees `{ id, name, url, has_auth }`.
//
// External MCP traffic flows through `/api/mcp/proxy/<id>` so the
// configured Authorization / X-API-Key / etc. headers attach
// server-side instead of in the browser.

export interface McpServerSummary {
  id: string;
  name: string;
  url: string;
  has_auth: boolean;
}

export interface McpServerInput {
  id: string;
  name: string;
  url: string;
  /** Optional. Stored on disk; never returned from the GET endpoint. */
  headers?: Record<string, string>;
}

export async function listMcpServers(): Promise<McpServerSummary[]> {
  const r = await fetch('/api/mcp/servers');
  const j = (await r.json()) as { servers?: McpServerSummary[]; error?: string };
  if (j.error) throw new Error(j.error);
  return j.servers ?? [];
}

export async function upsertMcpServer(entry: McpServerInput): Promise<void> {
  const r = await fetch('/api/mcp/servers', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(entry),
  });
  const j = (await r.json()) as { ok?: boolean; error?: string };
  if (j.error || !j.ok) throw new Error(j.error ?? 'failed to save MCP server');
}

export async function deleteMcpServer(id: string): Promise<void> {
  const r = await fetch('/api/mcp/servers/' + encodeURIComponent(id), { method: 'DELETE' });
  const j = (await r.json()) as { ok?: boolean; error?: string };
  if (j.error || !j.ok) throw new Error(j.error ?? 'failed to delete MCP server');
}

/** URL the browser hits to talk to a configured external MCP server.
 *  Auth headers attach server-side; this URL is otherwise drop-in for
 *  the existing McpClient. */
export function proxyUrlFor(serverId: string): string {
  return new URL(
    '/api/mcp/proxy/' + encodeURIComponent(serverId),
    window.location.origin,
  ).toString();
}
