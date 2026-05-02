// Render an arbitrary thrown value as a useful, user-visible string.
//
// `(e as Error).message` is fine when the throw site really threw an
// Error, but a lot of upstream code (fetch rejections, AbortError, plain
// object throws from libraries, JSON-parsed API errors) throws non-Error
// values whose `.message` is undefined. The fallback `String(e)` then
// produces "[object Object]" which is useless to the user.
//
// This helper handles every common shape and never returns
// "[object Object]". Use it in every catch block that surfaces an error
// to the chat UI.
export function errMessage(e: unknown): string {
  if (e == null) return 'Unknown error';
  if (typeof e === 'string') return e || 'Unknown error';
  if (e instanceof Error) {
    return e.message || e.name || 'Unknown error';
  }
  if (typeof e === 'object') {
    const obj = e as Record<string, unknown>;
    // Common nested shapes: {message: string}, {error: string},
    // {error: {message: string}}, {detail: string}, axios-style
    // {response: {data: {error: ...}}}.
    if (typeof obj.message === 'string' && obj.message) return obj.message;
    if (typeof obj.error === 'string' && obj.error) return obj.error;
    if (obj.error && typeof obj.error === 'object') {
      const inner = obj.error as Record<string, unknown>;
      if (typeof inner.message === 'string' && inner.message) return inner.message;
    }
    if (typeof obj.detail === 'string' && obj.detail) return obj.detail;
    if (obj.response && typeof obj.response === 'object') {
      const resp = obj.response as Record<string, unknown>;
      if (resp.data && typeof resp.data === 'object') {
        const data = resp.data as Record<string, unknown>;
        if (typeof data.error === 'string' && data.error) return data.error;
        if (typeof data.message === 'string' && data.message) return data.message;
      }
    }
    try {
      const s = JSON.stringify(e);
      if (s && s !== '{}' && s !== 'null') return s;
    } catch {
      /* fall through to String() */
    }
  }
  const s = String(e);
  return s && s !== '[object Object]' ? s : 'Unknown error';
}
