import React, { useState, useEffect } from 'react';
import type { WikiPageInfo, WikiPageDetail } from '../types/api';
import { fetchWikiDetail } from '../services/api';
import { Book, ChevronRight } from 'lucide-react';

interface WikiTabProps {
  wikiPages: WikiPageInfo[];
}

export const WikiTab: React.FC<WikiTabProps> = ({ wikiPages }) => {
  const [selectedFile, setSelectedFile] = useState<string | null>(null);
  const [detail, setDetail] = useState<WikiPageDetail | null>(null);
  const [loading, setLoading] = useState<boolean>(false);

  const activePage = selectedFile || wikiPages[0]?.file;

  useEffect(() => {
    if (activePage) {
      setLoading(true);
      fetchWikiDetail(activePage).then((res) => {
        setDetail(res);
        setLoading(false);
      });
    }
  }, [activePage]);

  return (
    <div className="space-y-6 animate-fade-in">
      <div className="glass-panel p-6 rounded-xl border border-gray-800 flex flex-col md:flex-row md:items-center justify-between gap-4">
        <div>
          <h2 className="text-xl font-bold text-white flex items-center gap-2">
            <Book className="text-emerald-400" size={24} />
            Auto-Generated Codebase Documentation Wiki
          </h2>
          <p className="text-sm text-gray-400 mt-1">
            Deterministic per-file documentation pages covering symbols, dependencies, and health findings.
          </p>
        </div>
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        {/* Page List Sidebar */}
        <div className="glass-panel p-5 rounded-xl border border-gray-800 space-y-3">
          <h3 className="text-sm font-semibold text-gray-300 uppercase tracking-wider">
            Wiki Pages ({wikiPages.length})
          </h3>
          <div className="space-y-2 max-h-[550px] overflow-y-auto pr-1">
            {wikiPages.length === 0 ? (
              <div className="p-4 text-xs text-gray-400 border border-dashed border-gray-800 rounded-lg text-center space-y-1">
                <div className="text-gray-300 font-medium">No Wiki Pages Generated</div>
                <div className="text-gray-500 font-mono">Run `repowise docs &lt;path&gt;` to build documentation wiki pages.</div>
              </div>
            ) : (
              wikiPages.map((p) => {
                const isSelected = activePage === p.file;
                return (
                  <button
                    key={p.file}
                    onClick={() => setSelectedFile(p.file)}
                    className={`w-full text-left p-3.5 rounded-lg border transition-all flex items-center justify-between ${
                      isSelected
                        ? 'bg-emerald-500/20 border-emerald-500/50 text-white font-semibold'
                        : 'bg-gray-900/50 border-gray-800/80 text-gray-300 hover:bg-gray-800/60'
                    }`}
                  >
                    <div className="truncate pr-2">
                      <div className="text-sm font-medium text-white truncate">{p.title}</div>
                      <div className="text-xs font-mono text-gray-400 truncate mt-0.5">{p.file}</div>
                    </div>
                    <ChevronRight size={16} className="text-gray-500 flex-shrink-0" />
                  </button>
                );
              })
            )}
          </div>
        </div>

        {/* Content Viewer */}
        <div className="glass-panel p-6 rounded-xl border border-gray-800 lg:col-span-2 space-y-4 min-h-[500px]">
          {loading ? (
            <div className="p-8 text-center text-gray-400">Loading page documentation...</div>
          ) : detail ? (
            <div>
              <div className="pb-4 border-b border-gray-800 flex items-center justify-between">
                <div>
                  <div className="text-xs font-mono text-emerald-400">{detail.file}</div>
                </div>
              </div>
              <div className="mt-6 prose prose-invert max-w-none text-gray-300 font-sans leading-relaxed whitespace-pre-wrap">
                {detail.content}
              </div>
            </div>
          ) : (
            <div className="p-8 text-center text-gray-400">Select a wiki page to view documentation.</div>
          )}
        </div>
      </div>
    </div>
  );
};
