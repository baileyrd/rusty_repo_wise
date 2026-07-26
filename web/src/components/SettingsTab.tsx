import React, { useState, useEffect } from 'react';
import type { SettingsData, UsageData, ReindexStatus } from '../types/api';
import { triggerReindex, fetchSettings, fetchUsage } from '../services/api';
import { Settings, RefreshCw, Cpu, HardDrive } from 'lucide-react';

interface SettingsTabProps {
  settings: SettingsData | null;
  usage: UsageData | null;
  hasRepos?: boolean;
}

export const SettingsTab: React.FC<SettingsTabProps> = ({ settings: initialSettings, usage: initialUsage, hasRepos = true }) => {
  const [settings, setSettings] = useState<SettingsData | null>(initialSettings);
  const [usage, setUsage] = useState<UsageData | null>(initialUsage);
  const [reindexing, setReindexing] = useState<boolean>(false);
  const [reindexMsg, setReindexMsg] = useState<string | null>(null);

  useEffect(() => {
    if (initialSettings) {
      setSettings(initialSettings);
    } else {
      fetchSettings().then(setSettings);
    }

    if (initialUsage) {
      setUsage(initialUsage);
    } else {
      fetchUsage().then(setUsage);
    }
  }, [initialSettings, initialUsage]);

  const handleReindex = async () => {
    setReindexing(true);
    setReindexMsg('Kicking off background re-index...');
    try {
      const res: ReindexStatus = await triggerReindex();
      setReindexMsg(`Reindex triggered: ${res.message || res.status}`);
      const freshSettings = await fetchSettings();
      setSettings(freshSettings);
    } catch {
      setReindexMsg('Failed to trigger re-index.');
    } finally {
      setTimeout(() => setReindexing(false), 2000);
    }
  };

  const activeSettings = hasRepos
    ? (settings || {
        repo_root: 'C:\\dev\\remind_me',
        file_count: 42,
        indexed_file_count: 42,
        has_git: true,
        has_wiki: false,
        llm_configured: false,
        llm_model: undefined,
      })
    : {
        repo_root: 'Not configured (No repository loaded)',
        file_count: 0,
        indexed_file_count: 0,
        has_git: false,
        has_wiki: false,
        llm_configured: false,
        llm_model: undefined,
      };

  const activeUsage = usage || {
    total_chat_calls: 0,
    prompt_tokens: 0,
    completion_tokens: 0,
    total_tokens: 0,
    estimated_cost_usd: 0,
  };

  return (
    <div className="space-y-6 animate-fade-in max-w-7xl mx-auto select-none">
      <div className="repowise-card p-6 rounded-xl border flex flex-col md:flex-row md:items-center justify-between gap-4">
        <div>
          <h2 className="text-xl font-bold text-gray-900 dark:text-white flex items-center gap-2">
            <Settings className="text-[#E67E22]" size={24} />
            System Settings & Token Usage
          </h2>
          <p className="text-xs text-gray-500 mt-1">
            Server configuration, active index metrics, and LLM token usage tracking.
          </p>
        </div>

        <button
          onClick={handleReindex}
          disabled={reindexing}
          className="px-4 py-2.5 rounded-xl bg-[#E67E22] hover:bg-[#D35400] text-white text-xs font-bold flex items-center gap-2 transition-colors disabled:opacity-50 shadow-sm"
        >
          <RefreshCw size={14} className={reindexing ? 'animate-spin' : ''} />
          <span>Trigger Re-index</span>
        </button>
      </div>

      {reindexMsg && (
        <div className="p-4 rounded-xl bg-blue-500/10 border border-blue-500/30 text-blue-500 text-xs font-mono flex items-center gap-2">
          <span>{reindexMsg}</span>
        </div>
      )}

      <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
        {/* Settings Info */}
        <div className="repowise-card p-6 rounded-xl border space-y-4">
          <h3 className="text-base font-bold text-gray-900 dark:text-white flex items-center gap-2">
            <HardDrive size={18} className="text-[#E67E22]" /> Repository Info
          </h3>

          <div className="space-y-3 text-xs font-mono">
            <div className="flex justify-between py-2 border-b border-gray-200 dark:border-gray-800">
              <span className="text-gray-500">Repo Root</span>
              <span className="text-gray-900 dark:text-white font-bold break-all max-w-[250px]">{activeSettings.repo_root}</span>
            </div>
            <div className="flex justify-between py-2 border-b border-gray-200 dark:border-gray-800">
              <span className="text-gray-500">Total Indexed Files</span>
              <span className="text-gray-900 dark:text-white font-bold">{activeSettings.indexed_file_count}</span>
            </div>
            <div className="flex justify-between py-2 border-b border-gray-200 dark:border-gray-800">
              <span className="text-gray-500">Git History Provider</span>
              <span className={activeSettings.has_git ? 'text-emerald-500 font-bold' : 'text-gray-400 font-bold'}>
                {activeSettings.has_git ? 'Available' : 'Unavailable'}
              </span>
            </div>
            <div className="flex justify-between py-2 border-b border-gray-200 dark:border-gray-800">
              <span className="text-gray-500">Documentation Wiki</span>
              <span className={activeSettings.has_wiki ? 'text-emerald-500 font-bold' : 'text-gray-400 font-bold'}>
                {activeSettings.has_wiki ? 'Generated' : 'Not generated'}
              </span>
            </div>
            <div className="flex justify-between py-2">
              <span className="text-gray-500">LLM Engine</span>
              <span className="text-purple-500 font-bold">
                {activeSettings.llm_configured ? activeSettings.llm_model || 'Configured' : 'Opt-in (Not configured)'}
              </span>
            </div>
          </div>
        </div>

        {/* LLM Usage Info */}
        <div className="repowise-card p-6 rounded-xl border space-y-4">
          <h3 className="text-base font-bold text-gray-900 dark:text-white flex items-center gap-2">
            <Cpu size={18} className="text-purple-500" /> LLM Cost & Usage Tracking
          </h3>

          <div className="space-y-3 text-xs font-mono">
            <div className="flex justify-between py-2 border-b border-gray-200 dark:border-gray-800">
              <span className="text-gray-500">Total Chat Calls</span>
              <span className="text-gray-900 dark:text-white font-bold">{activeUsage.total_chat_calls}</span>
            </div>
            <div className="flex justify-between py-2 border-b border-gray-200 dark:border-gray-800">
              <span className="text-gray-500">Prompt Tokens</span>
              <span className="text-cyan-500 font-bold">{activeUsage.prompt_tokens.toLocaleString()}</span>
            </div>
            <div className="flex justify-between py-2 border-b border-gray-200 dark:border-gray-800">
              <span className="text-gray-500">Completion Tokens</span>
              <span className="text-purple-500 font-bold">{activeUsage.completion_tokens.toLocaleString()}</span>
            </div>
            <div className="flex justify-between py-2 border-b border-gray-200 dark:border-gray-800">
              <span className="text-gray-500">Total Token Volume</span>
              <span className="text-emerald-500 font-bold">{activeUsage.total_tokens.toLocaleString()}</span>
            </div>
            <div className="flex justify-between py-2">
              <span className="text-gray-500">Estimated Cost (USD)</span>
              <span className="text-amber-500 font-bold">${activeUsage.estimated_cost_usd.toFixed(3)}</span>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};
