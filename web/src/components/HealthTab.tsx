import React, { useState } from 'react';
import type { HealthData, FileHealthScore } from '../types/api';
import { X } from 'lucide-react';

interface HealthTabProps {
  health: HealthData | null;
}

export const HealthTab: React.FC<HealthTabProps> = ({ health }) => {
  const [subTab, setSubTab] = useState<'overview' | 'findings' | 'hotspots' | 'coverage' | 'deadcode' | 'impact' | 'security'>('overview');
  const [selectedFile, setSelectedFile] = useState<FileHealthScore | null>(null);
  const [colorMode, setColorMode] = useState<'health' | 'maintainability' | 'performance'>('health');
  const [impactPaths] = useState<string[]>(['packages/server/', 'packages/core/']);

  const activeHealth = health || {
    overall_score: 8.4,
    defect_risk_score: 7.5,
    maintainability_score: 8.6,
    performance_risks: 268,
    open_findings: 8710,
    worst_files: [
      {
        file: 'packages/server/app.py',
        score: 5.1,
        lines: 480,
        churn: 142,
        findings: [
          { kind: 'long_function', file: 'packages/server/app.py', line: 112, symbol: 'parse_fn_body', penalty: 1.2, description: 'Function length (184 lines) exceeds threshold' },
          { kind: 'complex_conditional', file: 'packages/server/app.py', line: 245, symbol: 'parse_fn_body', penalty: 0.8, description: 'Condition chains 4 boolean operators' },
        ],
      },
      {
        file: 'serve_cmd.py',
        score: 1.0,
        lines: 240,
        churn: 64,
        findings: [
          { kind: 'hot_path_sync_io', file: 'serve_cmd.py', line: 88, symbol: 'main', penalty: 2.5, description: 'Blocking sync I/O in main thread' },
        ],
      },
    ],
    finding_counts_by_kind: [],
    coverage_stats: { files_instrumented: 1413, line_coverage_pct: 88.9, uncovered_lines: 12789 },
    dead_code_summary: { candidate_lines: 1651, high_confidence: 259, medium_confidence: 412 },
    security_findings: {
      high: 0,
      medium: 101,
      low: 96,
      items: [
        { file: 'tests/test_injection.py', line: 45, kind: 'security_sensitive_symbol', snippet: 'raw_eval_query()', severity: 'medium' },
      ],
    },
  };

  const subNav = [
    { id: 'overview', label: 'Overview' },
    { id: 'findings', label: 'Findings' },
    { id: 'hotspots', label: 'Hotspots' },
    { id: 'coverage', label: 'Coverage' },
    { id: 'deadcode', label: 'Dead Code' },
    { id: 'impact', label: 'Impact' },
    { id: 'security', label: 'Security' },
  ];

  return (
    <div className="space-y-6 animate-fade-in max-w-7xl mx-auto select-none">
      {/* Sub Navigation Bar */}
      <div className="repowise-card p-2 rounded-xl border flex items-center justify-between">
        <div className="flex space-x-1 overflow-x-auto">
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
      </div>

      {/* OVERVIEW SUB-TAB */}
      {subTab === 'overview' && (
        <div className="space-y-6">
          {/* Key KPI Cards */}
          <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
            <div className="repowise-card p-5 rounded-xl border">
              <div className="text-xs text-gray-500 font-bold uppercase">DEFECT RISK</div>
              <div className="flex items-baseline space-x-2 mt-1">
                <span className="text-3xl font-black font-mono text-amber-500">{activeHealth.defect_risk_score}</span>
                <span className="text-xs font-bold text-amber-500 uppercase px-2 py-0.5 rounded bg-amber-500/10 border">WARNING</span>
              </div>
              <div className="h-1.5 w-full bg-gray-200 dark:bg-gray-800 rounded-full mt-3 overflow-hidden">
                <div style={{ width: '75%' }} className="h-full bg-amber-500" />
              </div>
            </div>

            <div className="repowise-card p-5 rounded-xl border">
              <div className="text-xs text-gray-500 font-bold uppercase">MAINTAINABILITY</div>
              <div className="flex items-baseline space-x-2 mt-1">
                <span className="text-3xl font-black font-mono text-emerald-500">{activeHealth.maintainability_score}</span>
                <span className="text-xs font-bold text-emerald-500 uppercase px-2 py-0.5 rounded bg-emerald-500/10 border">HEALTHY</span>
              </div>
              <div className="text-xs text-gray-400 mt-2 font-mono">2,941 findings total</div>
            </div>

            <div className="repowise-card p-5 rounded-xl border">
              <div className="text-xs text-gray-500 font-bold uppercase">PERFORMANCE</div>
              <div className="flex items-baseline space-x-2 mt-1">
                <span className="text-3xl font-black font-mono text-gray-900 dark:text-white">{activeHealth.performance_risks}</span>
                <span className="text-xs font-bold text-emerald-500 uppercase px-2 py-0.5 rounded bg-emerald-500/10 border">HEALTHY</span>
              </div>
              <div className="text-xs text-gray-400 mt-2 font-mono">Risk score 9.9/10</div>
            </div>
          </div>

          {/* CODE HEALTH BUBBLE PACKING MAP */}
          <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
            <div className="repowise-card p-6 rounded-xl border lg:col-span-2 space-y-4">
              <div className="flex items-center justify-between">
                <div>
                  <h3 className="text-base font-bold text-gray-900 dark:text-white">Code Health Map</h3>
                  <div className="text-xs text-gray-500">2,802 files &bull; click a cluster to zoom, file to open</div>
                </div>

                <div className="flex items-center bg-gray-200 dark:bg-gray-800 p-0.5 rounded-lg border">
                  <button
                    onClick={() => setColorMode('health')}
                    className={`px-3 py-1 rounded text-xs font-bold ${colorMode === 'health' ? 'bg-white dark:bg-[#1C2128] text-[#E67E22]' : 'text-gray-500'}`}
                  >
                    Health
                  </button>
                  <button
                    onClick={() => setColorMode('maintainability')}
                    className={`px-3 py-1 rounded text-xs font-bold ${colorMode === 'maintainability' ? 'bg-white dark:bg-[#1C2128] text-[#E67E22]' : 'text-gray-500'}`}
                  >
                    Maintainability
                  </button>
                </div>
              </div>

              {/* Multi-Cluster Circle Packing Canvas */}
              <div className="w-full h-[480px] bg-[#FAF8F5] dark:bg-[#0E1117] rounded-xl border flex items-center justify-center relative overflow-hidden">
                <svg viewBox="0 0 600 450" className="w-full h-full">
                  {/* Large Cluster 1: PACKAGES */}
                  <g onClick={() => setSelectedFile(activeHealth.worst_files[0])} className="cursor-pointer">
                    <circle cx="260" cy="240" r="140" fill="#10B981" fillOpacity="0.25" stroke="#10B981" strokeWidth="2" />
                    <circle cx="210" cy="200" r="35" fill="#E67E22" fillOpacity="0.7" />
                    <circle cx="280" cy="280" r="28" fill="#F43F5E" fillOpacity="0.8" />
                    <circle cx="310" cy="180" r="40" fill="#10B981" fillOpacity="0.8" />
                    <text x="260" y="244" textAnchor="middle" fill="#FFFFFF" fontSize="13" fontFamily="monospace" fontWeight="bold">
                      PACKAGES
                    </text>
                  </g>

                  {/* Large Cluster 2: TESTS */}
                  <g onClick={() => setSelectedFile(activeHealth.worst_files[1])} className="cursor-pointer">
                    <circle cx="480" cy="240" r="85" fill="#10B981" fillOpacity="0.25" stroke="#10B981" strokeWidth="2" />
                    <circle cx="480" cy="240" r="30" fill="#10B981" fillOpacity="0.8" />
                    <text x="480" y="244" textAnchor="middle" fill="#FFFFFF" fontSize="12" fontFamily="monospace" fontWeight="bold">
                      TESTS
                    </text>
                  </g>
                </svg>
              </div>
            </div>

            {/* Side Findings List */}
            <div className="repowise-card p-6 rounded-xl border space-y-4">
              <h3 className="text-base font-bold text-gray-900 dark:text-white">Findings Queue</h3>
              <div className="space-y-2 max-h-[440px] overflow-y-auto pr-1">
                {activeHealth.worst_files.map((wf) => (
                  <div
                    key={wf.file}
                    onClick={() => setSelectedFile(wf)}
                    className="p-3 rounded-lg border bg-gray-50 dark:bg-[#1C2128] hover:border-[#E67E22] cursor-pointer space-y-1"
                  >
                    <div className="flex items-center justify-between">
                      <span className="font-mono text-xs font-bold text-blue-500 truncate max-w-[180px]">{wf.file}</span>
                      <span className="px-2 py-0.5 rounded text-[10px] font-mono font-bold bg-amber-500/10 text-amber-500 border border-amber-500/20">
                        {wf.score.toFixed(1)} / 10
                      </span>
                    </div>
                    <div className="text-[11px] text-gray-500 font-mono">
                      {wf.findings.length} penalties detected
                    </div>
                  </div>
                ))}
              </div>
            </div>
          </div>
        </div>
      )}

      {/* COVERAGE SUB-TAB */}
      {subTab === 'coverage' && (
        <div className="repowise-card p-6 rounded-xl border space-y-6">
          <div className="grid grid-cols-3 gap-4 text-center">
            <div className="p-4 rounded-xl border bg-gray-50 dark:bg-[#1C2128]">
              <div className="text-xs font-bold text-gray-400">FILES INSTRUMENTED</div>
              <div className="text-2xl font-black font-mono mt-1">1,413</div>
            </div>
            <div className="p-4 rounded-xl border bg-emerald-500/10 border-emerald-500/20 text-emerald-500">
              <div className="text-xs font-bold">LINE COVERAGE</div>
              <div className="text-2xl font-black font-mono mt-1">88.9%</div>
            </div>
            <div className="p-4 rounded-xl border bg-gray-50 dark:bg-[#1C2128]">
              <div className="text-xs font-bold text-gray-400">UNCOVERED LINES</div>
              <div className="text-2xl font-black font-mono text-rose-500 mt-1">12,789</div>
            </div>
          </div>
        </div>
      )}

      {/* IMPACT SUB-TAB */}
      {subTab === 'impact' && (
        <div className="repowise-card p-6 rounded-xl border space-y-6">
          <div>
            <h3 className="text-lg font-bold text-gray-900 dark:text-white">Impact Analyzer</h3>
            <p className="text-xs text-gray-500 mt-0.5">
              Estimate the blast radius of proposed file changes across dependencies and test suites.
            </p>
          </div>

          <div className="p-4 rounded-xl bg-gray-50 dark:bg-[#0E1117] border space-y-3">
            <div className="text-xs font-bold text-gray-400 uppercase">Target Modified Paths</div>
            <div className="flex flex-wrap gap-2">
              {impactPaths.map((p, idx) => (
                <span key={idx} className="px-3 py-1 rounded-lg bg-blue-500/10 text-blue-500 border border-blue-500/20 font-mono text-xs flex items-center gap-2">
                  {p}
                </span>
              ))}
            </div>
          </div>

          <div className="grid grid-cols-4 gap-4 text-center">
            <div className="p-4 rounded-xl border">
              <div className="text-xs text-gray-400 font-bold">AFFECTED FILES</div>
              <div className="text-2xl font-black text-gray-900 dark:text-white mt-1">2</div>
            </div>
            <div className="p-4 rounded-xl border">
              <div className="text-xs text-gray-400 font-bold">DOWNSTREAM</div>
              <div className="text-2xl font-black text-blue-500 mt-1">625</div>
            </div>
            <div className="p-4 rounded-xl border">
              <div className="text-xs text-gray-400 font-bold">CO-CHANGE PATHS</div>
              <div className="text-2xl font-black text-purple-500 mt-1">43</div>
            </div>
            <div className="p-4 rounded-xl border">
              <div className="text-xs text-gray-400 font-bold">TEST RISKS</div>
              <div className="text-2xl font-black text-amber-500 mt-1">124</div>
            </div>
          </div>
        </div>
      )}

      {/* SECURITY SUB-TAB */}
      {subTab === 'security' && (
        <div className="repowise-card p-6 rounded-xl border space-y-6">
          <div className="flex items-center justify-between">
            <h3 className="text-lg font-bold text-gray-900 dark:text-white">Security Scan Queue</h3>
            <span className="text-xs text-gray-500 font-mono">197 findings detected</span>
          </div>

          <div className="grid grid-cols-3 gap-4 text-center">
            <div className="p-4 rounded-xl border bg-rose-500/10 border-rose-500/20 text-rose-500">
              <div className="text-xs font-bold">HIGH SEVERITY</div>
              <div className="text-2xl font-black font-mono mt-1">0</div>
            </div>
            <div className="p-4 rounded-xl border bg-amber-500/10 border-amber-500/20 text-amber-500">
              <div className="text-xs font-bold">MEDIUM SEVERITY</div>
              <div className="text-2xl font-black font-mono mt-1">101</div>
            </div>
            <div className="p-4 rounded-xl border bg-blue-500/10 border-blue-500/20 text-blue-500">
              <div className="text-xs font-bold">LOW SEVERITY</div>
              <div className="text-2xl font-black font-mono mt-1">96</div>
            </div>
          </div>
        </div>
      )}

      {/* FILE INSPECTOR DRAWER */}
      {selectedFile && (
        <div className="fixed inset-y-0 right-0 z-50 w-96 bg-white dark:bg-[#161B22] border-l shadow-2xl p-6 space-y-6 animate-fade-in">
          <div className="flex items-center justify-between border-b pb-4">
            <div>
              <div className="text-xs font-mono font-bold text-[#E67E22]">{selectedFile.file}</div>
              <h2 className="text-lg font-extrabold text-gray-900 dark:text-white mt-1">File Health Inspector</h2>
            </div>
            <button onClick={() => setSelectedFile(null)} className="p-1 rounded-full hover:bg-gray-200 dark:hover:bg-gray-800">
              <X size={20} />
            </button>
          </div>

          <div className="grid grid-cols-2 gap-3 text-center">
            <div className="p-3 rounded-lg border bg-gray-50 dark:bg-[#0E1117]">
              <div className="text-[10px] font-bold text-gray-400">SCORE</div>
              <div className="text-xl font-black font-mono text-amber-500 mt-1">{selectedFile.score.toFixed(1)} / 10</div>
            </div>
            <div className="p-3 rounded-lg border bg-gray-50 dark:bg-[#0E1117]">
              <div className="text-[10px] font-bold text-gray-400">PENALTIES</div>
              <div className="text-xl font-black font-mono text-rose-500 mt-1">{selectedFile.findings.length}</div>
            </div>
          </div>

          <div className="space-y-3">
            <h4 className="text-xs font-bold text-gray-400 uppercase">Anti-Pattern Penalties</h4>
            {selectedFile.findings.map((f, idx) => (
              <div key={idx} className="p-3 rounded-lg border bg-gray-50 dark:bg-[#0E1117] space-y-1">
                <div className="flex items-center justify-between text-xs font-bold font-mono text-rose-500">
                  <span>{f.kind}</span>
                  <span>-{f.penalty} pts</span>
                </div>
                <div className="text-xs text-gray-600 dark:text-gray-400">{f.description}</div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
};
