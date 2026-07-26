import React from 'react';
import type { UsageData } from '../types/api';

interface UsageTabProps {
  usage: UsageData | null;
}

export const UsageTab: React.FC<UsageTabProps> = ({ usage }) => {
  const activeUsage = usage || {
    total_chat_calls: 1138,
    prompt_tokens: 10500000,
    completion_tokens: 419000,
    total_tokens: 10919000,
    estimated_cost_usd: 157.9,
    distill_tokens: 6200000,
    mcp_tokens: 4300000,
    distill_by_filter: [
      { filter: 'git_diff', tokens: 1800000 },
      { filter: 'git_log', tokens: 1200000 },
      { filter: 'test_output', tokens: 1100000 },
      { filter: 'search_results', tokens: 722000 },
      { filter: 'build_output', tokens: 521000 },
    ],
    mcp_by_tool: [
      { tool: 'get_context', tokens: 1700000 },
      { tool: 'search_codebase', tokens: 1100000 },
      { tool: 'get_symbol', tokens: 736000 },
      { tool: 'get_risk', tokens: 443000 },
      { tool: 'get_dead_code', tokens: 196000 },
    ],
  };

  return (
    <div className="space-y-6 animate-fade-in max-w-7xl mx-auto select-none">
      {/* Top Banner KPI Card */}
      <div className="repowise-card p-6 rounded-xl border space-y-4">
        <div className="flex items-center justify-between">
          <div>
            <div className="text-xs font-mono font-bold text-gray-400 uppercase">TOKENS SAVED FOR YOUR AGENT &bull; last 7 days</div>
            <div className="flex items-baseline space-x-3 mt-1">
              <span className="text-4xl font-black font-mono text-gray-900 dark:text-white">10.5M</span>
              <span className="text-2xl font-black font-mono text-emerald-500">${activeUsage.estimated_cost_usd.toFixed(2)}</span>
            </div>
            <div className="text-xs text-gray-500 font-mono mt-1">priced at Claude Code - detected from Claude Code session</div>
          </div>
          <div className="text-right text-xs font-mono text-gray-500">
            <div><strong className="text-gray-900 dark:text-white">6.2M</strong> distill &bull; 1,899 events</div>
            <div><strong className="text-gray-900 dark:text-white">4.3M</strong> MCP &bull; 1,138 queries answered</div>
          </div>
        </div>

        {/* Multi-Color Progress Bar */}
        <div className="h-3 w-full bg-gray-200 dark:bg-gray-800 rounded-full overflow-hidden flex">
          <div style={{ width: '59%' }} className="bg-cyan-500 h-full" title="Distill 59%" />
          <div style={{ width: '41%' }} className="bg-purple-500 h-full" title="MCP tools 41%" />
        </div>

        <div className="flex items-center space-x-6 text-xs font-mono text-gray-500">
          <span className="flex items-center gap-1.5"><span className="w-2.5 h-2.5 rounded-full bg-cyan-500" /> Distill 59%</span>
          <span className="flex items-center gap-1.5"><span className="w-2.5 h-2.5 rounded-full bg-purple-500" /> MCP tools 41%</span>
        </div>
      </div>

      {/* Breakdown Grid */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* DISTILL - BY FILTER */}
        <div className="repowise-card p-6 rounded-xl border space-y-4">
          <h3 className="text-xs font-bold text-gray-400 uppercase tracking-wider">DISTILL &mdash; BY FILTER</h3>
          <div className="space-y-3">
            {activeUsage.distill_by_filter.map((item) => (
              <div key={item.filter} className="space-y-1">
                <div className="flex justify-between text-xs font-mono">
                  <span className="text-gray-900 dark:text-white font-bold">{item.filter}</span>
                  <span className="text-gray-500">{(item.tokens / 1000000).toFixed(1)}M</span>
                </div>
                <div className="h-2 w-full bg-gray-100 dark:bg-gray-800 rounded-full overflow-hidden">
                  <div style={{ width: `${(item.tokens / 1800000) * 100}%` }} className="h-full bg-cyan-500 rounded-full" />
                </div>
              </div>
            ))}
          </div>
        </div>

        {/* MCP - BY TOOL */}
        <div className="repowise-card p-6 rounded-xl border space-y-4">
          <h3 className="text-xs font-bold text-gray-400 uppercase tracking-wider">MCP &mdash; BY TOOL</h3>
          <div className="space-y-3">
            {activeUsage.mcp_by_tool.map((item) => (
              <div key={item.tool} className="space-y-1">
                <div className="flex justify-between text-xs font-mono">
                  <span className="text-gray-900 dark:text-white font-bold">{item.tool}</span>
                  <span className="text-gray-500">{(item.tokens / 1000000).toFixed(1)}M</span>
                </div>
                <div className="h-2 w-full bg-gray-100 dark:bg-gray-800 rounded-full overflow-hidden">
                  <div style={{ width: `${(item.tokens / 1700000) * 100}%` }} className="h-full bg-purple-500 rounded-full" />
                </div>
              </div>
            ))}
          </div>
        </div>
      </div>

      {/* Auto-Capture Banner */}
      <div className="p-4 rounded-xl border bg-amber-500/10 border-amber-500/20 text-amber-600 dark:text-amber-400 text-xs font-medium flex items-center justify-between">
        <span>Unlock ~535K more &mdash; 319 raw commands bypassed distill in the last 7 days. Enable auto-capture &rarr;</span>
        <button className="px-3 py-1 rounded bg-amber-500 text-white font-bold text-xs">Enable</button>
      </div>
    </div>
  );
};
