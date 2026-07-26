import React from 'react';
import type { HotspotsData } from '../types/api';
import { Flame, GitCommit, Activity } from 'lucide-react';

interface HotspotsTabProps {
  hotspots: HotspotsData | null;
}

export const HotspotsTab: React.FC<HotspotsTabProps> = ({ hotspots }) => {
  if (!hotspots) {
    return <div className="p-8 text-center text-gray-400">Loading hotspot analytics...</div>;
  }

  const maxScore = Math.max(...hotspots.hotspots.map((h) => h.score), 1);

  return (
    <div className="space-y-6 animate-fade-in">
      <div className="glass-panel p-6 rounded-xl border border-gray-800 flex flex-col md:flex-row md:items-center justify-between gap-4">
        <div>
          <h2 className="text-xl font-bold text-white flex items-center gap-2">
            <Flame className="text-rose-400" size={24} />
            Git Hotspots & Churn Analytics
          </h2>
          <p className="text-sm text-gray-400 mt-1">
            Ranks files by Hotspot Score = (Git Commit Churn) &times; (Cyclomatic Code Complexity).
          </p>
        </div>
      </div>

      <div className="glass-panel p-6 rounded-xl border border-gray-800 space-y-4">
        <div className="flex items-center justify-between">
          <h3 className="text-lg font-semibold text-white">Top Risk Hotspot Files</h3>
          <span className="text-xs text-gray-400">{hotspots.hotspots.length} files analyzed</span>
        </div>

        <div className="space-y-3">
          {hotspots.hotspots.length === 0 ? (
            <div className="p-8 text-center text-gray-400 border border-dashed border-gray-800 rounded-xl space-y-2">
              <div className="text-white font-medium">No Git Churn Hotspots Detected</div>
              <div className="text-xs text-gray-500">
                Git history is either unavailable or has zero commit churn for this repository root.
              </div>
            </div>
          ) : (
            hotspots.hotspots.map((item, idx) => {
              const barPercentage = Math.min((item.score / maxScore) * 100, 100);
              return (
                <div key={item.file} className="p-4 rounded-xl bg-gray-900/80 border border-gray-800/80 space-y-3">
                  <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2">
                    <div className="flex items-center gap-3">
                      <span className="w-6 h-6 rounded-full bg-rose-500/10 border border-rose-500/30 text-rose-400 font-mono text-xs flex items-center justify-center font-bold">
                        #{idx + 1}
                      </span>
                      <span className="font-mono text-sm font-semibold text-blue-300 break-all">{item.file}</span>
                    </div>
                    <div className="flex items-center gap-4 text-xs font-mono">
                      <span className="flex items-center gap-1 text-gray-400">
                        <GitCommit size={14} className="text-purple-400" /> {item.churn} commits
                      </span>
                      <span className="flex items-center gap-1 text-gray-400">
                        <Activity size={14} className="text-cyan-400" /> {item.complexity} complexity
                      </span>
                      <span className="px-2.5 py-1 rounded bg-rose-500/10 text-rose-400 border border-rose-500/20 font-bold">
                        Score: {item.score.toFixed(1)}
                      </span>
                    </div>
                  </div>

                  <div className="h-2 w-full bg-gray-950 rounded-full overflow-hidden">
                    <div
                      style={{ width: `${barPercentage}%` }}
                      className="h-full bg-gradient-to-r from-blue-500 via-purple-500 to-rose-500 rounded-full"
                    />
                  </div>
                </div>
              );
            })
          )}
        </div>
      </div>
    </div>
  );
};
