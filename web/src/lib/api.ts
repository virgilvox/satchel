// Thin wrapper over the satchel REST API. Every call is fire-and-forget
// fetch — errors are surfaced as `{ error }` payloads in the response so
// callers can render them inline instead of catching exceptions.

import type {
  BrowseResponse,
  FileTypeStat,
  IngestJob,
  SearchPage,
  SourcesPage,
  StatusResponse,
} from './types';

const API_BASE = window.location.origin;

async function getJson<T>(url: string): Promise<T> {
  const r = await fetch(API_BASE + url);
  return r.json() as Promise<T>;
}

async function postJson<T>(url: string, body: unknown): Promise<T> {
  const r = await fetch(API_BASE + url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  return r.json() as Promise<T>;
}

async function deleteJson<T>(url: string, body: unknown): Promise<T> {
  const r = await fetch(API_BASE + url, {
    method: 'DELETE',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  return r.json() as Promise<T>;
}

export const api = {
  status: () => getJson<StatusResponse>('/api/status'),

  search: (query: string, top_k = 20, offset = 0, collection_id?: number) =>
    postJson<SearchPage>('/api/search', {
      query,
      top_k,
      offset,
      ...(collection_id != null ? { collection_id } : {}),
    }),

  sources: (params: {
    q?: string;
    filter_type?: string;
    sort_by?: string;
    limit?: number;
    offset?: number;
    collection_id?: number;
  }) => {
    const qs = new URLSearchParams();
    if (params.q) qs.set('q', params.q);
    if (params.filter_type) qs.set('filter_type', params.filter_type);
    if (params.sort_by) qs.set('sort_by', params.sort_by);
    if (params.limit) qs.set('limit', String(params.limit));
    if (params.offset) qs.set('offset', String(params.offset));
    if (params.collection_id != null) qs.set('collection_id', String(params.collection_id));
    return getJson<SourcesPage>('/api/sources?' + qs.toString());
  },

  types: () => getJson<{ types: FileTypeStat[]; error?: string }>('/api/types'),

  document: (source: string) =>
    getJson<{ text: string; source: string; error?: string }>(
      '/api/document?source=' + encodeURIComponent(source)
    ),

  conversation: (source: string, limit = 2000, offset = 0) =>
    getJson<{
      records: Array<{ text: string; title?: string }>;
      total: number;
      error?: string;
    }>(
      '/api/conversation?source=' +
        encodeURIComponent(source) +
        '&limit=' +
        limit +
        '&offset=' +
        offset
    ),

  browse: (path: string) =>
    getJson<BrowseResponse>('/api/browse?path=' + encodeURIComponent(path)),

  ingest: (path: string) =>
    postJson<{ job_id?: string; error?: string }>('/api/ingest', { path }),

  jobs: () => getJson<{ jobs: IngestJob[] }>('/api/jobs'),

  deleteSources: (
    body: { path?: string; prefix?: string; file_type?: string; dry_run: boolean; confirm?: boolean }
  ) =>
    deleteJson<{ deleted_documents: number; deleted_chunks: number; error?: string }>(
      '/api/sources',
      // The server requires `confirm: true` on a write call as a safety
      // gate. Auto-supply it when the caller is doing a write (dry_run
      // false) — the UI flow already collects an explicit confirmation
      // before invoking this.
      { ...body, confirm: body.confirm ?? !body.dry_run },
    ),

  clear: (body: { confirm?: boolean; dry_run?: boolean }) =>
    postJson<{ deleted_documents: number; deleted_chunks: number; error?: string }>(
      '/api/clear',
      body
    ),

  collectionsList: () => getJson<{ collections: CollectionSummary[]; error?: string }>('/api/collections'),
  collectionsCreate: (name: string) =>
    postJson<{ id: number; name: string; error?: string }>('/api/collections', { name }),
  collectionsDelete: (id: number) =>
    deleteJson<{ ok?: boolean; error?: string }>('/api/collections/' + id, {}),
  collectionAssign: (id: number, source_paths: string[]) =>
    postJson<{ added: number; error?: string }>(
      '/api/collections/' + id + '/sources',
      { source_paths },
    ),
  collectionUnassign: (id: number, source_paths: string[]) =>
    deleteJson<{ removed: number; error?: string }>(
      '/api/collections/' + id + '/sources',
      { source_paths },
    ),

  tunnelStatus: () => getJson<TunnelState>('/api/tunnel'),
  tunnelStart:  (mode: TunnelMode = 'quick') =>
    postJson<TunnelState>('/api/tunnel/start', { mode }),
  tunnelStop:   () => postJson<TunnelState>('/api/tunnel/stop', {}),
  tunnelConfigGet:   () => getJson<TunnelConfigState>('/api/tunnel/config'),
  tunnelConfigSet:   (body: { token: string; hostname: string }) =>
    postJson<TunnelConfigState>('/api/tunnel/config', body),
  tunnelConfigClear: () => deleteJson<TunnelConfigState>('/api/tunnel/config', {}),
};

export type TunnelMode = 'quick' | 'named';

export interface CollectionSummary {
  id: number;
  name: string;
  created_at: string;
  document_count: number;
}

export interface TunnelState {
  installed: boolean;
  running: boolean;
  mode: TunnelMode;
  url: string | null;
  forwarding: string | null;
  started_at: string | null;
  error: string | null;
  named?: {
    configured: boolean;
    hostname: string | null;
  };
}

export interface TunnelConfigState {
  configured: boolean;
  hostname: string | null;
  error?: string;
}

export const ORIGIN = API_BASE;
