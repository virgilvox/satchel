// Minimal MCP JSON-RPC client over HTTP. Targets the satchel server's /mcp
// endpoint by default but can be pointed at anything that speaks the same
// transport. The chat client uses this to expose vault tools to WebLLM.

import type { McpTool } from './types';

interface JsonRpcRequest {
  jsonrpc: '2.0';
  id: number;
  method: string;
  params?: Record<string, unknown>;
}

interface JsonRpcResponse {
  jsonrpc: '2.0';
  id: number;
  result?: unknown;
  error?: { code: number; message: string };
}

export class McpClient {
  private endpoint: string;
  private nextId = 1;
  private sessionId: string | null = null;

  constructor(endpoint = window.location.origin + '/mcp') {
    this.endpoint = endpoint;
  }

  setEndpoint(url: string) {
    this.endpoint = url;
    this.sessionId = null;
  }

  private async call<T>(method: string, params?: Record<string, unknown>): Promise<T> {
    const id = this.nextId++;
    const body: JsonRpcRequest = { jsonrpc: '2.0', id, method, params };
    const headers: Record<string, string> = { 'Content-Type': 'application/json' };
    if (this.sessionId) headers['mcp-session-id'] = this.sessionId;

    const r = await fetch(this.endpoint, {
      method: 'POST',
      headers,
      body: JSON.stringify(body),
    });

    const session = r.headers.get('mcp-session-id');
    if (session) this.sessionId = session;

    const text = await r.text();
    if (!text) throw new Error(`MCP ${method} returned empty body`);
    let envelope: JsonRpcResponse;
    try {
      envelope = JSON.parse(text);
    } catch (e) {
      throw new Error(`MCP ${method} returned non-JSON: ${text.slice(0, 200)}`);
    }
    if (envelope.error) {
      throw new Error(`MCP ${method} error: ${envelope.error.message}`);
    }
    return envelope.result as T;
  }

  async initialize(clientName = 'satchel-web', clientVersion = '0.1.0'): Promise<unknown> {
    return this.call('initialize', {
      protocolVersion: '2024-11-05',
      capabilities: {},
      clientInfo: { name: clientName, version: clientVersion },
    });
  }

  async listTools(): Promise<McpTool[]> {
    const result = await this.call<{ tools: McpTool[] }>('tools/list');
    return result.tools ?? [];
  }

  /**
   * Call a tool. Returns the textual content joined into a single string —
   * good enough for feeding back into LLM message history. Callers that
   * need the raw structured response should use {@link callRaw} instead.
   */
  async callTool(name: string, args: Record<string, unknown>): Promise<string> {
    const raw = await this.callRaw(name, args);
    if (!raw || typeof raw !== 'object') return JSON.stringify(raw);
    const c = (raw as { content?: Array<{ type: string; text?: string }> }).content;
    if (!Array.isArray(c)) return JSON.stringify(raw);
    return c
      .map((part) => (part.type === 'text' && part.text ? part.text : JSON.stringify(part)))
      .join('\n');
  }

  async callRaw(name: string, args: Record<string, unknown>): Promise<unknown> {
    return this.call('tools/call', { name, arguments: args });
  }
}
