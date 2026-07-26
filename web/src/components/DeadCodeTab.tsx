import React, { useState } from 'react';
import type { DeadCodeData } from '../types/api';
import { Trash2, AlertCircle, Filter } from 'lucide-react';

interface DeadCodeTabProps {
  deadCode: DeadCodeData | null;
}

export const DeadCodeTab: React.FC<DeadCodeTabProps> = ({ deadCode }) => {
  const [filterConfidence, setFilterConfidence] = useState<string>('all');

  if (!deadCode) {
    return <div className="p-8 text-center text-gray-400">Loading dead code candidates...</div>;
  }

  const filteredCandidates = deadCode.candidates.filter((c) => {
    if (filterConfidence === 'all') return true;
    return c.confidence === filterConfidence;
  });

  return (
    <div className="space-y-6 animate-fade-in">
      <div className="glass-panel p-6 rounded-xl border border-gray-800 flex flex-col md:flex-row md:items-center justify-between gap-4">
        <div>
          <h2 className="text-xl font-bold text-white flex items-center gap-2">
            <Trash2 className="text-purple-400" size={24} />
            Dead Code Candidate Analysis
          </h2>
          <p className="text-sm text-gray-400 mt-1">
            Confidence-tiered dead code detector identifying uncalled, unimported functions and symbols.
          </p>
        </div>

        <div className="flex items-center gap-2">
          <Filter size={16} className="text-gray-400" />
          <select
            value={filterConfidence}
            onChange={(e) => setFilterConfidence(e.target.value)}
            className="bg-gray-900 border border-gray-800 text-sm text-white rounded-lg px-3 py-1.5 focus:outline-none focus:border-purple-500"
          >
            <option value="all">All Confidence Levels ({deadCode.candidates.length})</option>
            <option value="high">High Confidence</option>
            <option value="medium">Medium Confidence</option>
            <option value="low">Low Confidence</option>
          </select>
        </div>
      </div>

      <div className="space-y-3">
        {filteredCandidates.length === 0 ? (
          <div className="glass-panel p-8 text-center text-gray-400 rounded-xl border border-gray-800">
            No dead code candidates found matching the selected filter.
          </div>
        ) : (
          filteredCandidates.map((c, idx) => {
            const badgeColor =
              c.confidence === 'high' ? 'bg-rose-500/10 text-rose-400 border-rose-500/30' :
              c.confidence === 'medium' ? 'bg-amber-500/10 text-amber-400 border-amber-500/30' :
              'bg-blue-500/10 text-blue-400 border-blue-500/30';
            return (
              <div key={idx} className="glass-panel p-5 rounded-xl border border-gray-800 space-y-3">
                <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2">
                  <div>
                    <span className="font-mono text-lg font-bold text-purple-300">{c.symbol_name}</span>
                    <div className="text-xs font-mono text-gray-400 mt-0.5">{c.file}:{c.line}</div>
                  </div>
                  <span className={`px-3 py-1 rounded-md text-xs font-bold uppercase border ${badgeColor}`}>
                    {c.confidence} Confidence
                  </span>
                </div>

                <div className="space-y-1.5">
                  {c.reasons.map((r, rIdx) => (
                    <div key={rIdx} className="text-xs text-gray-300 flex items-center gap-2">
                      <AlertCircle size={14} className="text-gray-500" />
                      <span>{r}</span>
                    </div>
                  ))}
                </div>
              </div>
            );
          })
        )}
      </div>
    </div>
  );
};
