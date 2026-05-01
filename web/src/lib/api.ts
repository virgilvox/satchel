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

  search: (query: string, top_k = 20, offset = 0) =>
    postJson<SearchPage>('/api/search', { query, top_k, offset }),

  sources: (params: {
    q?: string;
    filter_type?: string;
    sort_by?: string;
    limit?: number;
    offset?: number;
  }) => {
    const qs = new URLSearchParams();
    if (params.q) qs.set('q', params.q);
    if (params.filter_type) qs.set('filter_type', params.filter_type);
    if (params.sort_by) qs.set('sort_by', params.sort_by);
    if (params.limit) qs.set('limit', String(params.limit));
    if (params.offset) qs.set('offset', String(params.offset));
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
    body: { path?: string; prefix?: string; file_type?: string; dry_run: boolean }
  ) =>
    deleteJson<{ deleted_documents: number; deleted_chunks: number; error?: string }>(
      '/api/sources',
      body
    ),

  clear: (body: { confirm?: boolean; dry_run?: boolean }) =>
    postJson<{ deleted_documents: number; deleted_chunks: number; error?: string }>(
      '/api/clear',
      body
    ),
};

export const ORIGIN = API_BASE;
