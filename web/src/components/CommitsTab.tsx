import React from 'react';
import type { CommitsData } from '../types/api';
import { GitCommit } from 'lucide-react';

interface CommitsTabProps {
  commits?: CommitsData | null;
}

export const CommitsTab: React.FC<CommitsTabProps> = () => {
  const commitsList = [
    { hash: 'e8ce6e7', message: 'Initial commit: core architecture', author: 'Raghav Chamaliya', date: 'Mar 23', lines: 1850, risk: 'HIGH', category: 'feature' },
    { hash: 'fb535a0', message: 'UI: Airier, diagram-first web UI with a shared canvas', author: 'Sawat Ahuja', date: 'Apr 27', lines: 7200, risk: 'HIGH', category: 'fix' },
    { hash: '9b2c347', message: 'Refactor: decoupled file-indexed entities', author: 'Raghav Chamaliya', date: 'May 12', lines: 1200, risk: 'MEDIUM', category: 'refactor' },
  ];

  return (
    <div className="space-y-6 animate-fade-in max-w-7xl mx-auto select-none">
      <div className="repowise-card p-6 rounded-xl border flex items-center justify-between">
        <div>
          <h2 className="text-xl font-bold text-gray-900 dark:text-white flex items-center gap-2">
            <GitCommit size={24} className="text-[#E67E22]" /> Commits & History Analytics
          </h2>
          <p className="text-xs text-gray-500 mt-1">
            Review-priority queue &mdash; every indexed commit scored for change-risk and ranked relative to repo history.
          </p>
        </div>
      </div>

      {/* KPI Stats Row */}
      <div className="grid grid-cols-1 sm:grid-cols-4 gap-4 text-center">
        <div className="repowise-card p-5 rounded-xl border">
          <div className="text-xs font-bold text-gray-400">TOTAL COMMITS</div>
          <div className="text-2xl font-black font-mono text-gray-900 dark:text-white mt-1">485</div>
          <div className="text-[11px] text-gray-400 mt-0.5">with change-risk</div>
        </div>
        <div className="repowise-card p-5 rounded-xl border">
          <div className="text-xs font-bold text-rose-500">HIGH PRIORITY</div>
          <div className="text-2xl font-black font-mono text-rose-500 mt-1">172</div>
          <div className="text-[11px] text-gray-400 mt-0.5">1st risk quartile</div>
        </div>
        <div className="repowise-card p-5 rounded-xl border">
          <div className="text-xs font-bold text-amber-500">FIX COMMITS</div>
          <div className="text-2xl font-black font-mono text-amber-500 mt-1">141</div>
          <div className="text-[11px] text-gray-400 mt-0.5">bug-fix subjects</div>
        </div>
        <div className="repowise-card p-5 rounded-xl border">
          <div className="text-xs font-bold text-purple-500">AVG ENTROPY</div>
          <div className="text-2xl font-black font-mono text-purple-500 mt-1">2.12</div>
          <div className="text-[11px] text-gray-400 mt-0.5">change-diffusion</div>
        </div>
      </div>

      {/* Code Evolution Stacked Area Visualization Container */}
      <div className="repowise-card p-6 rounded-xl border space-y-4">
        <div className="flex items-center justify-between">
          <h3 className="text-base font-bold text-gray-900 dark:text-white">Code Evolution Timeline</h3>
          <div className="flex items-center space-x-3 text-xs font-mono">
            <span className="flex items-center gap-1"><span className="w-2.5 h-2.5 rounded-full bg-[#E67E22]" /> Feature 35%</span>
            <span className="flex items-center gap-1"><span className="w-2.5 h-2.5 rounded-full bg-amber-500" /> Fix 29%</span>
            <span className="flex items-center gap-1"><span className="w-2.5 h-2.5 rounded-full bg-purple-500" /> Refactor 10%</span>
            <span className="flex items-center gap-1"><span className="w-2.5 h-2.5 rounded-full bg-emerald-500" /> Docs 6%</span>
          </div>
        </div>

        <div className="w-full h-44 bg-[#FAF8F5] dark:bg-[#0E1117] rounded-xl border border-gray-200 dark:border-gray-800 flex items-center justify-center p-4">
          <svg viewBox="0 0 700 120" className="w-full h-full">
            <path d="M 0 100 Q 150 20 350 70 T 700 30 L 700 120 L 0 120 Z" fill="#E67E22" fillOpacity="0.6" />
            <path d="M 0 110 Q 180 40 350 90 T 700 60 L 700 120 L 0 120 Z" fill="#D97706" fillOpacity="0.7" />
          </svg>
        </div>
      </div>

      {/* Commits Queue Table */}
      <div className="repowise-card p-6 rounded-xl border space-y-3">
        <h3 className="text-base font-bold text-gray-900 dark:text-white">Review-Priority Queue</h3>
        <div className="space-y-2">
          {commitsList.map((c) => (
            <div key={c.hash} className="p-4 rounded-xl border bg-gray-50 dark:bg-[#1C2128] flex items-center justify-between">
              <div>
                <div className="font-mono text-sm font-bold text-gray-900 dark:text-white">{c.message}</div>
                <div className="text-xs font-mono text-gray-500 mt-1">{c.author} &bull; {c.date}</div>
              </div>
              <div className="flex items-center space-x-3 text-xs font-mono">
                <span className="text-emerald-500 font-bold">+{c.lines} lines</span>
                <span className="px-2.5 py-1 rounded font-bold uppercase bg-rose-500/10 text-rose-500 border border-rose-500/20">
                  {c.risk}
                </span>
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
};
