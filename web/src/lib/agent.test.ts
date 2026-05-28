// Unit tests for the v2.9.0 smart-mode helpers in agent.ts. Vitest
// runs from web/ via `npm test`; the tests live here so a regression
// in the agent loop's bound-checking, stall detection, or transcript
// compaction trips before users see it.

import { describe, it, expect } from 'vitest';
import {
  approximateTokens,
  truncateToolResult,
  hashToolCall,
  detectStallPattern,
  compactTranscript,
  stallNudgeToolResult,
  contextFullNudgeToolResult,
  compactSystemPrompt,
  parseConstrainedOutput,
  type CompactItem,
} from './agent';

describe('approximateTokens', () => {
  it('returns 0 for empty', () => {
    expect(approximateTokens('')).toBe(0);
  });
  it('rounds up to the nearest token (4 chars/token)', () => {
    expect(approximateTokens('a')).toBe(1);
    expect(approximateTokens('abcd')).toBe(1);
    expect(approximateTokens('abcde')).toBe(2);
  });
});

describe('truncateToolResult', () => {
  it('returns input verbatim when under budget', () => {
    const short = 'short result';
    expect(truncateToolResult(short, { maxTokens: 100 })).toBe(short);
  });
  it('caps output at the budget and appends a recoverable marker', () => {
    const big = 'x'.repeat(8000); // 2000 tokens
    const out = truncateToolResult(big, { maxTokens: 200 });
    // Budget should hold within a single-token margin (rounding).
    expect(approximateTokens(out)).toBeLessThanOrEqual(201);
    expect(out).toContain('truncated');
    expect(out.startsWith('x')).toBe(true); // head preserved
  });
  it('embeds the recoverHint when provided', () => {
    const out = truncateToolResult('y'.repeat(4000), {
      maxTokens: 100,
      recoverHint: 'get_chunk_context(chunk_id: "abc:3")',
    });
    expect(out).toContain('get_chunk_context');
  });
  it('handles empty input cleanly', () => {
    expect(truncateToolResult('', { maxTokens: 100 })).toBe('');
  });
});

describe('hashToolCall', () => {
  it('is order-independent for object keys', () => {
    const h1 = hashToolCall('search_knowledge', { query: 'x', top_k: 5 });
    const h2 = hashToolCall('search_knowledge', { top_k: 5, query: 'x' });
    expect(h1).toBe(h2);
  });
  it('distinguishes different tool names', () => {
    expect(hashToolCall('a', { x: 1 })).not.toBe(hashToolCall('b', { x: 1 }));
  });
  it('distinguishes different argument values', () => {
    expect(hashToolCall('a', { x: 1 })).not.toBe(hashToolCall('a', { x: 2 }));
  });
  it('handles null/undefined args', () => {
    // Should not throw; should be deterministic.
    const h1 = hashToolCall('a', undefined);
    const h2 = hashToolCall('a', null);
    expect(h1).toBe(h2);
  });
});

describe('detectStallPattern', () => {
  it('returns ok with no history', () => {
    const d = detectStallPattern({
      toolHashHistory: [],
      contextUsedTokens: 0,
      contextWindowTokens: 8192,
    });
    expect(d.kind).toBe('ok');
  });
  it('fires duplicate when last two hashes match', () => {
    const d = detectStallPattern({
      toolHashHistory: ['a', 'b', 'b'],
      contextUsedTokens: 1000,
      contextWindowTokens: 8192,
    });
    expect(d.kind).toBe('duplicate');
  });
  it('does NOT fire duplicate when the prior pair differs', () => {
    const d = detectStallPattern({
      toolHashHistory: ['a', 'b', 'c'],
      contextUsedTokens: 1000,
      contextWindowTokens: 8192,
    });
    expect(d.kind).toBe('ok');
  });
  it('fires context-full when used / window >= threshold', () => {
    const d = detectStallPattern({
      toolHashHistory: ['a'],
      contextUsedTokens: 7000,
      contextWindowTokens: 8192, // ~85%
    });
    expect(d.kind).toBe('context-full');
  });
  it('respects an explicit contextFullFraction override', () => {
    const d = detectStallPattern({
      toolHashHistory: ['a'],
      contextUsedTokens: 5000,
      contextWindowTokens: 8192, // ~61%
      contextFullFraction: 0.5,
    });
    expect(d.kind).toBe('context-full');
  });
  it('treats a zero window as "no signal" (no context-full trigger)', () => {
    const d = detectStallPattern({
      toolHashHistory: ['a', 'a'],
      contextUsedTokens: 999_999,
      contextWindowTokens: 0,
    });
    // With no window we cannot tell; duplicate still wins.
    expect(d.kind).toBe('duplicate');
  });
});

describe('compactTranscript', () => {
  function item(role: CompactItem['role'], text: string, isToolPair = false, summary?: string): CompactItem {
    return {
      role,
      text,
      tokens: approximateTokens(text),
      isToolPair,
      summary,
    };
  }

  it('returns input verbatim when under budget', () => {
    const t: CompactItem[] = [item('system', 'sys'), item('user', 'hi')];
    expect(compactTranscript(t, { budgetTokens: 100 })).toBe(t);
  });

  it('does NOT collapse user / system messages', () => {
    const big = 'x'.repeat(4000);
    const t: CompactItem[] = [
      item('system', big),
      item('user', big),
      item('assistant', big, true, 'summary-A'),
    ];
    const out = compactTranscript(t, { budgetTokens: 100, keepRecentToolPairs: 0 });
    // System + user preserved; tool-pair item collapsed.
    expect(out[0].text).toBe(big);
    expect(out[1].text).toBe(big);
    expect(out[2].text).toContain('summary-A');
  });

  it('keeps the most recent K tool pairs verbatim', () => {
    const t: CompactItem[] = [
      item('user', 'q'),
      item('assistant', 'oldest', true, 'old-summary'),
      item('assistant', 'middle', true, 'mid-summary'),
      item('assistant', 'recent', true, 'recent-summary'),
    ];
    const out = compactTranscript(t, { budgetTokens: 1, keepRecentToolPairs: 2 });
    // Oldest pair (index 1) should be collapsed; middle + recent kept.
    expect(out[1].text).toContain('old-summary');
    expect(out[2].text).toBe('middle');
    expect(out[3].text).toBe('recent');
  });

  it('stops dropping once the budget is satisfied', () => {
    const t: CompactItem[] = [
      item('assistant', 'x'.repeat(4000), true, 'sum1'),
      item('assistant', 'x'.repeat(4000), true, 'sum2'),
      item('assistant', 'x'.repeat(4000), true, 'sum3'),
      item('assistant', 'x'.repeat(4000), true, 'sum4'),
    ];
    // Each pair is ~1000 tokens. Total ~4000. Budget 2200; should drop
    // enough to fit but not more than necessary.
    const out = compactTranscript(t, { budgetTokens: 2200, keepRecentToolPairs: 0 });
    const total = out.reduce((s, it) => s + it.tokens, 0);
    expect(total).toBeLessThanOrEqual(2300); // budget + one collapsed item's marker
  });
});

describe('stallNudgeToolResult / contextFullNudgeToolResult', () => {
  it('stall nudge mentions varying the query and respond_to_user', () => {
    const m = stallNudgeToolResult();
    expect(m).toMatch(/respond_to_user/);
    expect(m).toMatch(/different/i);
  });
  it('context-full nudge surfaces the percentage', () => {
    expect(contextFullNudgeToolResult(0.76)).toContain('76%');
    expect(contextFullNudgeToolResult(0.5)).toContain('50%');
  });
});

describe('compactSystemPrompt', () => {
  it('lists each tool with a one-liner', () => {
    const out = compactSystemPrompt({
      tools: [
        { name: 'search_knowledge', description: 'Search the vault. Returns chunks.' },
        { name: 'get_document', description: 'Fetch one doc.' },
      ],
      minToolCalls: 1,
    });
    expect(out).toContain('search_knowledge');
    expect(out).toContain('Search the vault');
    expect(out).toContain('get_document');
    expect(out).toContain('respond_to_user');
  });
  it('stays under ~600 tokens for a typical 7-tool vault', () => {
    const tools = Array.from({ length: 7 }, (_, i) => ({
      name: 'tool_' + i,
      description: 'Does the thing #' + i + '. Lorem ipsum.',
    }));
    const out = compactSystemPrompt({ tools, minToolCalls: 1 });
    expect(approximateTokens(out)).toBeLessThan(600);
  });
});

describe('parseConstrainedOutput integration', () => {
  it('still parses a respond_to_user envelope (pre-v2.9.0 regression guard)', () => {
    const out = parseConstrainedOutput(
      JSON.stringify({
        thought: 'done',
        tool_call: { name: 'respond_to_user', arguments: { answer: 'yes' } },
      }),
    );
    expect(out.answer).toBe('yes');
    expect(out.toolCalls).toHaveLength(0);
  });
  it('still parses a tool_call envelope (regression guard)', () => {
    const out = parseConstrainedOutput(
      JSON.stringify({
        thought: 'searching',
        tool_call: { name: 'search_knowledge', arguments: { query: 'x' } },
      }),
    );
    expect(out.toolCalls[0]?.name).toBe('search_knowledge');
    expect(out.answer).toBeNull();
  });
});
