import React from 'react';
import type { WorkspaceData } from '../types/api';
import { Layers, Folder, CheckCircle, AlertTriangle, GitFork, ArrowRight, Plus, Trash2 } from 'lucide-react';

interface WorkspaceTabProps {
  workspace: WorkspaceData | null;
  onSelectRepo?: (repoName: string) => void;
  onOpenAddRepoModal?: () => void;
  onClearWorkspace?: () => void;
}

export const WorkspaceTab: React.FC<WorkspaceTabProps> = ({
  workspace,
  onSelectRepo,
  onOpenAddRepoModal,
  onClearWorkspace,
}) => {
  const activeWorkspace = workspace || { repos: [] };

  return (
    <div className="space-y-6 animate-fade-in max-w-7xl mx-auto select-none">
      {/* Header Banner */}
      <div className="p-6 rounded-xl border bg-white dark:bg-[#1C2128] border-gray-200 dark:border-[#2D333B] flex flex-col md:flex-row md:items-center justify-between gap-4 shadow-sm">
        <div>
          <h2 className="text-xl font-bold text-gray-900 dark:text-white flex items-center gap-2">
            <Layers className="text-[#E67E22]" size={24} />
            Multi-Repository Workspace Intelligence
          </h2>
          <p className="text-xs text-gray-600 dark:text-gray-400 mt-1">
            Cross-repository dependencies, co-change coupling, and contract compliance across workspace repositories.
          </p>
        </div>

        <div className="flex items-center space-x-3">
          <span className="px-3 py-1 rounded-full text-xs font-mono font-bold bg-[#E67E22]/10 text-[#E67E22] border border-[#E67E22]/20">
            {activeWorkspace.repos.length} Repositories Configured
          </span>

          {activeWorkspace.repos.length > 0 && onClearWorkspace && (
            <button
              onClick={onClearWorkspace}
              title="Clear loaded repos to view blank state"
              className="px-3 py-1 rounded-xl text-xs font-bold bg-rose-500/10 text-rose-600 dark:text-rose-400 hover:bg-rose-500/20 border border-rose-500/20 flex items-center gap-1.5 transition-colors"
            >
              <Trash2 size={13} /> View Blank State
            </button>
          )}
        </div>
      </div>

      {/* ZERO REPOS BLANK STATE SCREEN */}
      {activeWorkspace.repos.length === 0 ? (
        <div className="p-12 text-center rounded-2xl border bg-white dark:bg-[#1C2128] border-gray-200 dark:border-[#2D333B] space-y-4 shadow-sm my-8">
          <div className="w-14 h-14 rounded-2xl bg-amber-500/10 text-[#E67E22] flex items-center justify-center mx-auto border border-[#E67E22]/20">
            <Folder size={28} />
          </div>
          <div className="space-y-1">
            <h3 className="text-lg font-bold text-gray-900 dark:text-white">No Repositories Loaded</h3>
            <p className="text-xs text-gray-600 dark:text-gray-400 max-w-md mx-auto">
              Your workspace is currently blank. Pass <code className="px-1.5 py-0.5 rounded bg-gray-100 dark:bg-gray-800 font-mono text-[11px] text-[#E67E22]">--workspace &lt;path.toml&gt;</code> when launching <code className="px-1.5 py-0.5 rounded bg-gray-100 dark:bg-gray-800 font-mono text-[11px] text-[#E67E22]">repowise serve-dashboard</code>, or click below to index your first repository.
            </p>
          </div>
          {onOpenAddRepoModal && (
            <button
              onClick={onOpenAddRepoModal}
              className="px-5 py-2.5 rounded-xl text-xs font-bold bg-[#E67E22] hover:bg-[#D35400] text-white transition-all shadow-sm inline-flex items-center gap-2"
            >
              <Plus size={16} /> Add Your First Repository
            </button>
          )}
        </div>
      ) : (
        /* Repository Cards Grid */
        <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
          {activeWorkspace.repos.map((repo) => (
            <div
              key={repo.name}
              onClick={() => onSelectRepo && onSelectRepo(repo.name)}
              className="p-6 rounded-xl border bg-white dark:bg-[#1C2128] border-gray-200 dark:border-[#2D333B] space-y-4 hover:border-[#E67E22] cursor-pointer transition-all shadow-sm"
            >
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <div className="p-2.5 rounded-lg bg-blue-500/10 border border-blue-500/20 text-blue-500">
                    <Folder size={20} />
                  </div>
                  <div>
                    <h3 className="text-base font-bold text-gray-900 dark:text-white">{repo.name}</h3>
                    <div className="text-[11px] font-mono text-gray-500 dark:text-gray-400 mt-0.5 truncate max-w-[160px]">{repo.path}</div>
                  </div>
                </div>

                {repo.indexed ? (
                  <span className="px-2.5 py-1 rounded text-[10px] font-bold bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border border-emerald-500/20 flex items-center gap-1">
                    <CheckCircle size={12} /> Indexed
                  </span>
                ) : (
                  <span className="px-2.5 py-1 rounded text-[10px] font-bold bg-amber-500/10 text-amber-600 dark:text-amber-400 border border-amber-500/20 flex items-center gap-1">
                    <AlertTriangle size={12} /> Unindexed
                  </span>
                )}
              </div>

              <div className="pt-3 border-t border-gray-100 dark:border-gray-800 flex items-center justify-between text-xs text-gray-600 dark:text-gray-400 font-mono">
                <span>Indexed Files:</span>
                <strong className="text-gray-900 dark:text-white font-bold">{repo.file_count || 0} files</strong>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Cross-Repo Dependency Graph Coupling Section */}
      {activeWorkspace.repos.length > 0 && (
        <div className="p-6 rounded-xl border bg-white dark:bg-[#1C2128] border-gray-200 dark:border-[#2D333B] space-y-4 shadow-sm">
          <div className="flex items-center justify-between">
            <h3 className="text-base font-bold text-gray-900 dark:text-white flex items-center gap-2">
              <GitFork size={18} className="text-[#E67E22]" /> Cross-Repository Dependency Edges
            </h3>
            <span className="text-xs font-mono text-gray-500 dark:text-gray-400">258 cross-boundary import edges</span>
          </div>

          <div className="space-y-3">
            {[
              { from: 'repowise-web', to: 'repowise-server', count: 48, kind: 'HTTP API / REST' },
              { from: 'frontend', to: 'backend', count: 124, kind: 'GraphQL & Proto' },
              { from: 'repowise-cli', to: 'repowise-core', count: 86, kind: 'Rust use import' },
            ].map((edge, idx) => (
              <div key={idx} className="p-4 rounded-xl border bg-gray-50 dark:bg-[#0E1117] border-gray-200 dark:border-gray-800 flex items-center justify-between">
                <div className="flex items-center space-x-3 text-xs font-mono">
                  <span className="font-bold text-[#E67E22]">{edge.from}</span>
                  <ArrowRight size={14} className="text-gray-400" />
                  <span className="font-bold text-blue-500">{edge.to}</span>
                </div>

                <div className="flex items-center space-x-4 text-xs font-mono">
                  <span className="px-2.5 py-1 rounded bg-purple-500/10 text-purple-600 dark:text-purple-400 font-bold border border-purple-500/20">
                    {edge.kind}
                  </span>
                  <span className="text-gray-600 dark:text-gray-400">{edge.count} calls</span>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
};
