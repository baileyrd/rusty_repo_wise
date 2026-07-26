import React from 'react';
import type { OverviewData, HealthData, HotspotsData } from '../types/api';
import { Activity, ShieldAlert, Cpu, DollarSign, GitCommit, ArrowRight } from 'lucide-react';

interface OverviewTabProps {
  overview: OverviewData | null;
  health: HealthData | null;
  hotspots: HotspotsData | null;
  onSelectTab: (tabId: string) => void;
}

export const OverviewTab: React.FC<OverviewTabProps> = ({
  overview,
  health,
  onSelectTab,
}) => {
  const activeOverview = overview || {
    file_count: 109,
    total_lines: 52531,
    health_score: 8.4,
    risk_count: 269,
    tokens_saved: 9800000,
    saved_dollars: 147.0,
    authorship: [
      { author: 'Raghav', percentage: 76 },
      { author: 'axponus', percentage: 1 },
      { author: 'Sawat Ahuja', percentage: 28 },
      { author: 'AI Agent', percentage: 5 },
    ],
    symbol_counts: [],
    languages: [],
    recent_commits: [
      { hash: '71d1f518', message: 'feat(nav-tabs): optional leading icon on shared tab row', author: 'Primary Author', time_ago: '1h ago' },
      { hash: '16d7a419', message: 'feat(ui): promote the architecture tour trigger into response-distill', author: 'Primary Author', time_ago: '2h ago' },
      { hash: 'e45903b4', message: 'feat(graph-node): controlled color mode on the shared graph canvas', author: 'Primary Author', time_ago: '3h ago' },
    ],
    recent_decisions: [
      { id: 'ADR-001', title: 'Consolidated the MCP tool surface. Removed 6 redundant tool calls.', status: 'proposed', type: 'adr' },
      { id: 'ADR-002', title: 'Airier, diagram-first web UI. Overhauled dashboard on a shared canvas.', status: 'proposed', type: 'adr' },
    ],
  };

  const healthScore = health?.overall_score ?? activeOverview.health_score;

  return (
    <div className="space-y-6 animate-fade-in max-w-7xl mx-auto select-none">
      {/* Top Banner KPI Cards */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
        <div className="repowise-card p-5 rounded-xl border flex items-center justify-between shadow-sm">
          <div>
            <div className="text-xs font-bold uppercase tracking-wider text-gray-500 dark:text-gray-400">TOTAL INDEXED LINES</div>
            <div className="text-2xl font-black font-mono text-gray-900 dark:text-white mt-1">
              {activeOverview.total_lines.toLocaleString()}
            </div>
            <div className="text-xs text-gray-600 dark:text-gray-300 mt-1 font-mono font-semibold">{activeOverview.file_count.toLocaleString()} files</div>
          </div>
          <div className="p-3 rounded-lg bg-blue-500/10 text-blue-600 dark:text-blue-400 border border-blue-500/20">
            <Activity size={22} />
          </div>
        </div>

        <div className="repowise-card p-5 rounded-xl border flex items-center justify-between shadow-sm">
          <div>
            <div className="text-xs font-bold uppercase tracking-wider text-gray-500 dark:text-gray-400">HEALTH SCORE</div>
            <div className="flex items-baseline space-x-2 mt-1">
              <span className="text-2xl font-black font-mono text-emerald-600 dark:text-emerald-400">
                {healthScore.toFixed(1)}
              </span>
              <span className="text-xs font-bold uppercase text-emerald-600 dark:text-emerald-400 px-2 py-0.5 rounded bg-emerald-500/10 border border-emerald-500/20">
                EXCELLENT
              </span>
            </div>
            <div className="text-xs text-gray-600 dark:text-gray-300 mt-1 font-mono font-semibold">Worst file: serve_cmd.py (1.0)</div>
          </div>
          <div className="p-3 rounded-lg bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border border-emerald-500/20">
            <ShieldAlert size={22} />
          </div>
        </div>

        <div className="repowise-card p-5 rounded-xl border flex items-center justify-between shadow-sm">
          <div>
            <div className="text-xs font-bold uppercase tracking-wider text-gray-500 dark:text-gray-400">PERFORMANCE RISKS</div>
            <div className="text-2xl font-black font-mono text-gray-900 dark:text-white mt-1">
              {activeOverview.risk_count} <span className="text-xs text-gray-600 dark:text-gray-300 font-bold">risks</span>
            </div>
            <div className="text-xs text-gray-600 dark:text-gray-300 mt-1 font-mono font-semibold">Risk score 9.9/10 Healthy</div>
          </div>
          <div className="p-3 rounded-lg bg-amber-500/10 text-amber-600 dark:text-amber-400 border border-amber-500/20">
            <Cpu size={22} />
          </div>
        </div>

        <div className="repowise-card p-5 rounded-xl border flex items-center justify-between shadow-sm">
          <div>
            <div className="text-xs font-bold uppercase tracking-wider text-gray-500 dark:text-gray-400">SAVINGS (LAST 7 DAYS)</div>
            <div className="text-2xl font-black font-mono text-gray-900 dark:text-white mt-1">
              {(activeOverview.tokens_saved / 1000000).toFixed(1)}M <span className="text-sm text-emerald-600 dark:text-emerald-400 font-bold">${activeOverview.saved_dollars.toFixed(2)}</span>
            </div>
            <div className="text-xs text-gray-600 dark:text-gray-300 mt-1 font-mono font-semibold">Priced at Claude Code agent rate</div>
          </div>
          <div className="p-3 rounded-lg bg-purple-500/10 text-purple-600 dark:text-purple-400 border border-purple-500/20">
            <DollarSign size={22} />
          </div>
        </div>
      </div>

      {/* Authorship Bar Section */}
      <div className="repowise-card p-6 rounded-xl border space-y-3 shadow-sm">
        <div className="flex items-center justify-between">
          <div className="text-sm font-bold text-gray-900 dark:text-white flex items-center gap-2">
            <span>Contributors Breakdown</span>
            <span className="text-xs text-[#E67E22] font-mono cursor-pointer hover:underline" onClick={() => onSelectTab('contributors')}>View owners &rarr;</span>
          </div>
          <div className="text-xs text-gray-600 dark:text-gray-300 font-mono font-semibold">5% agent-written</div>
        </div>

        {/* Stacked Progress Bar */}
        <div className="h-3 w-full bg-gray-200 dark:bg-gray-800 rounded-full overflow-hidden flex">
          <div style={{ width: '76%' }} className="bg-[#E67E22] h-full" title="Raghav: 76%" />
          <div style={{ width: '1%' }} className="bg-cyan-500 h-full" title="axponus: 1%" />
          <div style={{ width: '18%' }} className="bg-emerald-500 h-full" title="Sawat Ahuja: 28%" />
          <div style={{ width: '5%' }} className="bg-purple-500 h-full" title="AI Agent: 5%" />
        </div>

        <div className="flex flex-wrap gap-4 text-xs font-mono font-semibold text-gray-700 dark:text-gray-300 pt-1">
          <span className="flex items-center gap-1.5"><span className="w-2.5 h-2.5 rounded-full bg-[#E67E22]" /> Raghav 76%</span>
          <span className="flex items-center gap-1.5"><span className="w-2.5 h-2.5 rounded-full bg-cyan-500" /> axponus 1%</span>
          <span className="flex items-center gap-1.5"><span className="w-2.5 h-2.5 rounded-full bg-emerald-500" /> Sawat Ahuja 28%</span>
          <span className="flex items-center gap-1.5"><span className="w-2.5 h-2.5 rounded-full bg-purple-500" /> 5% agent-written</span>
        </div>
      </div>

      {/* Main Grid: Pulse Activity & Decisions */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* Recent Commits Pulse */}
        <div className="repowise-card p-6 rounded-xl border space-y-4 shadow-sm">
          <div className="flex items-center justify-between pb-3 border-b border-gray-200 dark:border-gray-800">
            <h3 className="text-base font-bold text-gray-900 dark:text-white flex items-center gap-2">
              <GitCommit size={18} className="text-[#E67E22]" />
              Recent Commits
            </h3>
            <button onClick={() => onSelectTab('commits')} className="text-xs text-[#E67E22] hover:underline font-bold">
              View all &rarr;
            </button>
          </div>

          <div className="space-y-3">
            {activeOverview.recent_commits.map((c) => (
              <div key={c.hash} className="p-3.5 rounded-xl bg-gray-100 dark:bg-[#0E1117] border border-gray-200 dark:border-gray-800 flex items-center justify-between">
                <div className="truncate pr-2">
                  <div className="text-xs font-mono text-gray-900 dark:text-gray-100 font-bold truncate">{c.message}</div>
                  <div className="text-[11px] text-gray-600 dark:text-gray-400 font-mono mt-0.5 font-semibold">{c.author} &bull; {c.time_ago}</div>
                </div>
                <span className="px-2.5 py-1 rounded-lg font-mono text-[10px] font-bold bg-gray-200 dark:bg-gray-800 text-gray-800 dark:text-gray-200 border border-gray-300 dark:border-gray-700 shrink-0">
                  {c.hash}
                </span>
              </div>
            ))}
          </div>
        </div>

        {/* Recent Decisions Feed */}
        <div className="repowise-card p-6 rounded-xl border space-y-4 shadow-sm">
          <div className="flex items-center justify-between pb-3 border-b border-gray-200 dark:border-gray-800">
            <h3 className="text-base font-bold text-gray-900 dark:text-white flex items-center gap-2">
              <ArrowRight size={18} className="text-blue-500" />
              Recent Decisions
            </h3>
            <button onClick={() => onSelectTab('decisions')} className="text-xs text-blue-500 hover:underline font-bold">
              View all &rarr;
            </button>
          </div>

          <div className="space-y-3">
            {activeOverview.recent_decisions.map((d) => (
              <div key={d.id} className="p-3.5 rounded-xl bg-gray-100 dark:bg-[#0E1117] border border-gray-200 dark:border-gray-800 space-y-1">
                <div className="flex items-center justify-between">
                  <span className="font-mono text-xs font-extrabold text-[#E67E22]">{d.id}</span>
                  <span className="px-2.5 py-1 rounded-lg text-[10px] font-bold uppercase bg-blue-500/10 text-blue-600 dark:text-blue-400 border border-blue-500/20">
                    {d.status}
                  </span>
                </div>
                <div className="text-xs font-bold text-gray-900 dark:text-gray-100">{d.title}</div>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
};
