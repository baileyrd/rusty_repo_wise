import React, { useState } from 'react';
import type { GraphData, SymbolItem } from '../types/api';
import { Network, Search, X } from 'lucide-react';

interface ArchitectureTabProps {
  graph: GraphData | null;
  symbols: SymbolItem[];
}

export const ArchitectureTab: React.FC<ArchitectureTabProps> = ({ graph: _graph, symbols }) => {
  const [subTab, setSubTab] = useState<'communities' | 'explore' | 'coupling' | 'dependencies' | 'symbols'>('explore');
  const [selectedSymbol, setSelectedSymbol] = useState<SymbolItem | null>(null);
  const [symbolSearch, setSymbolSearch] = useState<string>('');

  const subNav = [
    { id: 'communities', label: 'Communities' },
    { id: 'explore', label: 'Explore' },
    { id: 'coupling', label: 'Coupling' },
    { id: 'dependencies', label: 'Dependencies' },
    { id: 'symbols', label: 'Symbols' },
  ];

  const packagesList = [
    { name: 'Next', version: '15.1.0', target: 'packages/web/package.json', ecosystem: 'npm' },
    { name: 'React', version: '19.0.0', target: 'packages/api-client/package.json', ecosystem: 'npm' },
    { name: 'Anthropic', version: '0.28.0', target: 'packages/cli/pyproject.toml', ecosystem: 'pypi' },
    { name: 'OpenAI', version: '1.58.0', target: 'pyproject.toml', ecosystem: 'pypi' },
    { name: 'FastAPI', version: '0.115.0', target: 'packages/server/pyproject.toml', ecosystem: 'pypi' },
    { name: 'Pydantic', version: '2.10.0', target: 'packages/core/pyproject.toml', ecosystem: 'pypi' },
  ];

  const filteredSymbols = symbols.filter((s) =>
    s.name.toLowerCase().includes(symbolSearch.toLowerCase()) || s.file.toLowerCase().includes(symbolSearch.toLowerCase())
  );

  return (
    <div className="space-y-6 animate-fade-in max-w-7xl mx-auto select-none">
      {/* Sub Navigation Bar */}
      <div className="repowise-card p-2 rounded-xl border flex items-center justify-between">
        <div className="flex space-x-1">
          {subNav.map((item) => (
            <button
              key={item.id}
              onClick={() => setSubTab(item.id as any)}
              className={`px-4 py-2 rounded-lg text-xs font-bold transition-all ${
                subTab === item.id
                  ? 'bg-[#E67E22]/15 text-[#E67E22] font-extrabold shadow-sm'
                  : 'text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-800'
              }`}
            >
              {item.label}
            </button>
          ))}
        </div>

        <div className="text-xs font-mono text-gray-400 px-3 hidden sm:block">
          Showing 1,500 of 20,126 by importance
        </div>
      </div>

      {/* SUB-TAB 1: EXPLORE (FORCE NETWORK GRAPH) */}
      {subTab === 'explore' && (
        <div className="repowise-card p-6 rounded-xl border space-y-4">
          <div className="flex items-center justify-between">
            <h3 className="text-base font-bold text-gray-900 dark:text-white flex items-center gap-2">
              <Network size={18} className="text-[#E67E22]" /> Dependency Explorer Canvas
            </h3>
            <span className="text-xs font-mono text-gray-400">1500 nodes &bull; 3719 edges</span>
          </div>

          <div className="w-full h-[550px] bg-[#FAF8F5] dark:bg-[#0E1117] rounded-xl border border-gray-200 dark:border-gray-800 relative flex items-center justify-center overflow-hidden">
            {/* SVG Interactive Canvas */}
            <svg viewBox="0 0 800 500" className="w-full h-full">
              {/* Cluster Background Nodes */}
              <circle cx="400" cy="250" r="140" fill="#E67E22" fillOpacity="0.08" stroke="#E67E22" strokeDasharray="4 4" strokeWidth="1" />
              <circle cx="220" cy="180" r="90" fill="#2563EB" fillOpacity="0.08" stroke="#2563EB" strokeDasharray="4 4" strokeWidth="1" />
              <circle cx="580" cy="300" r="110" fill="#10B981" fillOpacity="0.08" stroke="#10B981" strokeDasharray="4 4" strokeWidth="1" />

              {/* Connecting Lines */}
              <line x1="400" y1="250" x2="220" y2="180" stroke="#E67E22" strokeWidth="1.5" strokeOpacity="0.4" />
              <line x1="400" y1="250" x2="580" y2="300" stroke="#10B981" strokeWidth="1.5" strokeOpacity="0.4" />
              <line x1="220" y1="180" x2="580" y2="300" stroke="#2563EB" strokeWidth="1" strokeOpacity="0.2" />

              {/* Nodes */}
              <g className="cursor-pointer" onClick={() => setSelectedSymbol(symbols[0])}>
                <circle cx="400" cy="250" r="22" fill="#E67E22" stroke="#FFFFFF" strokeWidth="2" />
                <text x="400" y="284" textAnchor="middle" fill="#E67E22" fontSize="11" fontFamily="monospace" fontWeight="bold">
                  repowise/core/tree.py
                </text>
              </g>

              <g className="cursor-pointer" onClick={() => setSelectedSymbol(symbols[1])}>
                <circle cx="220" cy="180" r="16" fill="#2563EB" stroke="#FFFFFF" strokeWidth="2" />
                <text x="220" y="210" textAnchor="middle" fill="#2563EB" fontSize="10" fontFamily="monospace">
                  repowise/graph.py
                </text>
              </g>

              <g className="cursor-pointer" onClick={() => setSelectedSymbol(symbols[2])}>
                <circle cx="580" cy="300" r="18" fill="#10B981" stroke="#FFFFFF" strokeWidth="2" />
                <text x="580" y="332" textAnchor="middle" fill="#10B981" fontSize="10" fontFamily="monospace">
                  repowise/server.py
                </text>
              </g>
            </svg>
          </div>
        </div>
      )}

      {/* SUB-TAB 2: COUPLING (CIRCULAR CHORD DIAGRAM) */}
      {subTab === 'coupling' && (
        <div className="repowise-card p-6 rounded-xl border space-y-4">
          <div>
            <h3 className="text-lg font-bold text-gray-900 dark:text-white">Change Coupling Chord Diagram</h3>
            <p className="text-xs text-gray-500 mt-0.5">
              Files that tend to change together in the same commit. Temporal risk mapping for hidden file dependencies.
            </p>
          </div>

          <div className="w-full h-[520px] bg-[#FAF8F5] dark:bg-[#0E1117] rounded-xl border border-gray-200 dark:border-gray-800 flex items-center justify-center relative">
            <svg viewBox="0 0 500 500" className="w-full h-full max-w-lg max-h-lg">
              {/* Outer Circular Ring */}
              <circle cx="250" cy="250" r="180" fill="none" stroke="#EBE6DC" className="dark:stroke-gray-800" strokeWidth="12" />
              
              {/* Inner Connected Curves (Chord Ribbons) */}
              <path d="M 250 70 Q 250 250 380 330" fill="none" stroke="#E67E22" strokeWidth="2.5" strokeOpacity="0.7" />
              <path d="M 120 330 Q 250 250 380 330" fill="none" stroke="#2563EB" strokeWidth="2" strokeOpacity="0.5" />
              <path d="M 250 70 Q 250 250 120 330" fill="none" stroke="#10B981" strokeWidth="2" strokeOpacity="0.6" />

              {/* Outer Segment Labels */}
              <text x="250" y="52" textAnchor="middle" fill="#E67E22" fontSize="11" fontFamily="monospace" fontWeight="bold">PACKAGES</text>
              <text x="410" y="350" textAnchor="start" fill="#2563EB" fontSize="11" fontFamily="monospace" fontWeight="bold">TESTS</text>
              <text x="90" y="350" textAnchor="end" fill="#10B981" fontSize="11" fontFamily="monospace" fontWeight="bold">DOCS</text>
            </svg>
          </div>
        </div>
      )}

      {/* SUB-TAB 3: DEPENDENCIES */}
      {subTab === 'dependencies' && (
        <div className="repowise-card p-6 rounded-xl border space-y-4">
          <div className="flex items-center justify-between">
            <h3 className="text-lg font-bold text-gray-900 dark:text-white">Dependencies Manifest Table</h3>
            <span className="text-xs text-gray-500 font-mono">119 runtime &bull; 60 dev</span>
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
            {packagesList.map((pkg) => (
              <div key={pkg.name} className="p-4 rounded-xl border bg-gray-50 dark:bg-[#1C2128] space-y-2">
                <div className="flex items-center justify-between">
                  <span className="font-bold text-gray-900 dark:text-white text-base">{pkg.name}</span>
                  <span className="px-2 py-0.5 rounded text-xs font-mono font-bold bg-[#E67E22]/10 text-[#E67E22]">
                    v{pkg.version}
                  </span>
                </div>
                <div className="text-xs font-mono text-gray-500 truncate">{pkg.target}</div>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* SUB-TAB 4: SYMBOLS & SYMBOL DETAIL INSPECTION MODAL */}
      {subTab === 'symbols' && (
        <div className="repowise-card p-6 rounded-xl border space-y-4">
          <div className="flex items-center justify-between gap-4">
            <div className="relative flex-1 max-w-md">
              <Search className="absolute left-3 top-2.5 text-gray-400" size={16} />
              <input
                type="text"
                value={symbolSearch}
                onChange={(e) => setSymbolSearch(e.target.value)}
                placeholder="Search symbols..."
                className="w-full bg-gray-100 dark:bg-[#0E1117] border border-gray-300 dark:border-gray-800 rounded-lg pl-9 pr-4 py-2 text-xs text-gray-900 dark:text-white focus:outline-none"
              />
            </div>
            <span className="text-xs font-mono text-gray-500">Showing {filteredSymbols.length} of 22,046</span>
          </div>

          <div className="space-y-2">
            {filteredSymbols.map((sym) => (
              <div
                key={sym.id}
                onClick={() => setSelectedSymbol(sym)}
                className="p-3 rounded-lg border bg-gray-50 dark:bg-[#1C2128] hover:border-[#E67E22] cursor-pointer flex items-center justify-between transition-all"
              >
                <div>
                  <div className="font-mono text-sm font-bold text-[#E67E22]">{sym.name}</div>
                  <div className="text-xs font-mono text-gray-500 mt-0.5">{sym.file}</div>
                </div>
                <div className="flex items-center space-x-4 text-xs font-mono">
                  <span className="text-gray-500">Complexity: <strong className="text-gray-900 dark:text-white">{sym.complexity}</strong></span>
                  <span className="px-2 py-0.5 rounded bg-blue-500/10 text-blue-500 font-bold uppercase">{sym.kind}</span>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* SYMBOL DETAIL INSPECTION MODAL */}
      {selectedSymbol && (
        <div className="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4 animate-fade-in">
          <div className="repowise-card p-6 rounded-2xl border max-w-xl w-full bg-white dark:bg-[#161B22] space-y-6 shadow-2xl">
            <div className="flex items-center justify-between border-b pb-4">
              <div>
                <span className="px-2 py-0.5 rounded text-[10px] font-bold uppercase bg-blue-500/10 text-blue-500">
                  {selectedSymbol.kind}
                </span>
                <h2 className="text-xl font-extrabold text-gray-900 dark:text-white font-mono mt-1">
                  {selectedSymbol.name}
                </h2>
              </div>
              <button onClick={() => setSelectedSymbol(null)} className="p-1 rounded-full hover:bg-gray-200 dark:hover:bg-gray-800">
                <X size={20} />
              </button>
            </div>

            <div className="grid grid-cols-3 gap-3 text-center">
              <div className="p-3 rounded-lg bg-gray-100 dark:bg-[#0E1117] border">
                <div className="text-[10px] font-bold text-gray-400 uppercase">IMPORTANCE</div>
                <div className="text-lg font-black font-mono text-[#E67E22] mt-0.5">{selectedSymbol.pagerank_score ?? 0.584}</div>
              </div>
              <div className="p-3 rounded-lg bg-gray-100 dark:bg-[#0E1117] border">
                <div className="text-[10px] font-bold text-gray-400 uppercase">COMPLEXITY</div>
                <div className="text-lg font-black font-mono text-gray-900 dark:text-white mt-0.5">{selectedSymbol.complexity ?? 21}</div>
              </div>
              <div className="p-3 rounded-lg bg-gray-100 dark:bg-[#0E1117] border">
                <div className="text-[10px] font-bold text-gray-400 uppercase">MODIFICATIONS</div>
                <div className="text-lg font-black font-mono text-purple-500 mt-0.5">{selectedSymbol.modifications ?? 4}</div>
              </div>
            </div>

            <div className="space-y-2 text-xs font-mono text-gray-600 dark:text-gray-400 pt-2 border-t">
              <div className="flex justify-between">
                <span>Top Contributor:</span>
                <strong className="text-gray-900 dark:text-white">{selectedSymbol.author ?? 'Raghav Chamaliya'} (100%)</strong>
              </div>
              <div className="flex justify-between">
                <span>Source Path:</span>
                <span className="text-[#E67E22] truncate max-w-[280px]">{selectedSymbol.file}</span>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};
