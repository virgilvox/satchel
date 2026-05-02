// Shared types used across the satchel web UI.

export type Mode = 'dark' | 'light';

export type Tab =
  | 'dashboard'
  | 'ask'
  | 'chat'
  | 'search'
  | 'documents'
  | 'ingest'
  | 'manage'
  | 'connect';

export interface VaultStats {
  documents: number;
  chunks: number;
  dimensions: number;
  db_size: string;
}

export interface StatusResponse {
  status: string;
  version: string;
  embedding_model: string;
  embedding_available: boolean;
  stats?: VaultStats;
}

export interface SearchResult {
  source: string;
  text: string;
  score: number;
  // Server may include richer fields; treat extras as opaque.
  [key: string]: unknown;
}

export interface SearchPage {
  results: SearchResult[];
  total: number;
  offset: number;
  limit: number;
  error?: string;
}

export interface SourceRow {
  path: string;
  file_type: string;
  record_count: number;
  chunk_count: number;
  ingested_at?: string;
}

export interface SourcesPage {
  sources: SourceRow[];
  total: number;
  offset: number;
  limit: number;
  error?: string;
}

export interface DocumentRow {
  id: string;
  source_path: string;
  title: string | null;
  file_type: string;
  chunk_count: number;
  ingested_at: string;
  collection_ids: number[];
}

export interface DocumentsPage {
  documents: DocumentRow[];
  total: number;
  offset: number;
  limit: number;
  error?: string;
}

export interface FileTypeStat {
  file_type: string;
  source_count: number;
}

export type JobStatus = 'pending' | 'running' | 'completed' | 'failed';

export interface IngestJob {
  id: string;
  path: string;
  status: JobStatus;
  archive_kind?: string;
  files_seen: number;
  records_added: number;
  records_skipped: number;
  records_failed: number;
  current_file?: string;
  started_at?: string;
  finished_at?: string;
  error?: string;
}

export interface BrowseEntry {
  name: string;
  kind: 'dir' | 'file';
  path: string;
}

export interface BrowseResponse {
  path: string;
  parent?: string;
  entries: BrowseEntry[];
  error?: string;
}

// MCP tool descriptor as returned by the satchel server.
export interface McpTool {
  name: string;
  description: string;
  inputSchema?: Record<string, unknown>;
}

// One turn in the chat transcript.
export type ChatRole = 'user' | 'assistant' | 'tool' | 'system' | 'error';

export interface ToolCallSpec {
  id: string;
  name: string;
  args: Record<string, unknown>;
}

export interface ToolCallResult {
  id: string;
  name: string;
  args: Record<string, unknown>;
  result?: string;
  error?: string;
  pending: boolean;
}

export interface ChatMessage {
  id: string;
  role: ChatRole;
  // The assistant's user-facing text, post-reasoning, post-tool-calls.
  content: string;
  // The reasoning trace, if the model emitted one inside <think>...</think>.
  reasoning?: string;
  // Tool calls the assistant emitted in this turn.
  toolCalls?: ToolCallResult[];
  // Source attribution for an assistant message produced by the Ask tab
  // (which is pure retrieval, no LLM).
  retrieval?: SearchResult[];
  // True while the model is still streaming. Receivers can render a caret.
  streaming?: boolean;
}
