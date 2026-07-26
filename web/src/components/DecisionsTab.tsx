import React, { useState } from 'react';
import type { DecisionsData } from '../types/api';
import { BookOpen, GitCommit, MessageSquare, FileText } from 'lucide-react';

interface DecisionsTabProps {
  decisions: DecisionsData | null;
}

export const DecisionsTab: React.FC<DecisionsTabProps> = ({ decisions }) => {
  const [filterSource, setFilterSource] = useState<string>('all');

  if (!decisions) {
    return <div className="p-8 text-center text-gray-400">Loading architectural decisions...</div>;
  }

  const filteredDecisions = decisions.decisions.filter((d) => {
    if (filterSource === 'all') return true;
    return d.source === filterSource;
  });

  return (
    <div className="space-y-6 animate-fade-in">
      <div className="glass-panel p-6 rounded-xl border border-gray-800 flex flex-col md:flex-row md:items-center justify-between gap-4">
        <div>
          <h2 className="text-xl font-bold text-white flex items-center gap-2">
            <BookOpen className="text-blue-400" size={24} />
            Architectural Decision Records (ADRs) & History
          </h2>
          <p className="text-sm text-gray-400 mt-1">
            5-source mining across ADR files, decision commits, code comments, and pull requests.
          </p>
        </div>

        <div className="flex items-center gap-2">
          <select
            value={filterSource}
            onChange={(e) => setFilterSource(e.target.value)}
            className="bg-gray-900 border border-gray-800 text-sm text-white rounded-lg px-3 py-1.5 focus:outline-none focus:border-blue-500"
          >
            <option value="all">All Sources ({decisions.decisions.length})</option>
            <option value="adr">ADR Documents</option>
            <option value="commit">Commit Messages</option>
            <option value="comment">Code Comments</option>
          </select>
        </div>
      </div>

      <div className="space-y-4">
        {filteredDecisions.length === 0 ? (
          <div className="glass-panel p-8 text-center text-gray-400 rounded-xl border border-gray-800">
            No architectural decisions found for this filter.
          </div>
        ) : (
          filteredDecisions.map((d) => {
            const icon =
              d.source === 'adr' ? <FileText size={18} className="text-blue-400" /> :
              d.source === 'commit' ? <GitCommit size={18} className="text-purple-400" /> :
              <MessageSquare size={18} className="text-emerald-400" />;

            return (
              <div key={d.id} className="glass-panel p-6 rounded-xl border border-gray-800 space-y-3">
                <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2">
                  <div className="flex items-center gap-3">
                    <div className="p-2 rounded-lg bg-gray-900 border border-gray-800">{icon}</div>
                    <div>
                      <span className="text-xs font-mono font-bold text-gray-500">{d.id}</span>
                      <h3 className="text-base font-bold text-white mt-0.5">{d.title}</h3>
                    </div>
                  </div>
                  <span className="px-2.5 py-1 rounded text-xs font-mono font-bold uppercase bg-blue-500/10 text-blue-400 border border-blue-500/20">
                    {d.source}
                  </span>
                </div>

                <p className="text-sm text-gray-300 leading-relaxed">{d.summary}</p>

                {d.linked_files.length > 0 && (
                  <div className="pt-2 border-t border-gray-800/80 space-y-1">
                    <div className="text-xs text-gray-400 font-medium">Linked Target Files:</div>
                    <div className="flex flex-wrap gap-2">
                      {d.linked_files.map((file, fIdx) => (
                        <span key={fIdx} className="px-2.5 py-0.5 rounded bg-gray-900 text-xs font-mono text-cyan-300 border border-gray-800 flex items-center gap-1">
                          {file}
                        </span>
                      ))}
                    </div>
                  </div>
                )}
              </div>
            );
          })
        )}
      </div>
    </div>
  );
};
