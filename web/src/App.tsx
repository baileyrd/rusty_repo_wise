import { useState, useEffect } from 'react';
import type {
  OverviewData,
  HealthData,
  HotspotsData,
  DecisionsData,
  GraphData,
  WikiPageInfo,
  SearchResult,
  SettingsData,
  UsageData,
  SymbolsData,
} from './types/api';
import {
  checkServerOnline,
  fetchOverview,
  fetchHealth,
  fetchHotspots,
  fetchDecisions,
  fetchGraph,
  fetchWikiPages,
  fetchSettings,
  fetchUsage,
  fetchSymbols,
  searchCodebase,
} from './services/api';
import { Sidebar } from './components/Sidebar';
import { OverviewTab } from './components/OverviewTab';
import { DocsTab } from './components/DocsTab';
import { ArchitectureTab } from './components/ArchitectureTab';
import { GraphTab } from './components/GraphTab';
import { HealthTab } from './components/HealthTab';
import { CommitsTab } from './components/CommitsTab';
import { DecisionsTab } from './components/DecisionsTab';
import { ChatTab } from './components/ChatTab';
import { UsageTab } from './components/UsageTab';
import { WorkspaceTab } from './components/WorkspaceTab';
import { SettingsTab } from './components/SettingsTab';
import { AddRepoModal } from './components/AddRepoModal';
import type { WorkspaceRepoItem } from './components/Sidebar';
import { Search, X, CheckCircle2, AlertCircle } from 'lucide-react';

export function App() {
  const [activeTab, setActiveTab] = useState<string>('overview');
  const [selectedRepo, setSelectedRepo] = useState<string>('repowise');
  const [theme, setTheme] = useState<'light' | 'dark'>('light');
  const [isServerOnline, setIsServerOnline] = useState<boolean>(false);

  // Data States
  const [overview, setOverview] = useState<OverviewData | null>(null);
  const [health, setHealth] = useState<HealthData | null>(null);
  const [hotspots, setHotspots] = useState<HotspotsData | null>(null);
  const [decisions, setDecisions] = useState<DecisionsData | null>(null);
  const [graph, setGraph] = useState<GraphData | null>(null);
  const [wikiPages, setWikiPages] = useState<WikiPageInfo[]>([]);
  const [settings, setSettings] = useState<SettingsData | null>(null);
  const [usage, setUsage] = useState<UsageData | null>(null);
  const [symbols, setSymbols] = useState<SymbolsData | null>(null);

  // Search Modal State
  const [searchQuery, setSearchQuery] = useState<string>('');
  const [searchResults, setSearchResults] = useState<SearchResult[]>([]);
  const [isSearchOpen, setIsSearchOpen] = useState<boolean>(false);

  // Add Repo Modal State
  const [isAddRepoOpen, setIsAddRepoOpen] = useState<boolean>(false);
  const [customRepos, setCustomRepos] = useState<WorkspaceRepoItem[]>([
    { name: 'repowise', file_count: 2802, is_indexed: true, indexed: true, path: 'c:/dev/rusty_repo_wise' },
    { name: 'frontend', file_count: 412, is_indexed: true, indexed: true, path: 'c:/dev/repowise-frontend' },
    { name: 'backend', file_count: 856, is_indexed: true, indexed: true, path: 'c:/dev/repowise-backend' },
  ]);

  const handleAddRepo = (newRepo: WorkspaceRepoItem) => {
    setCustomRepos((prev) => [...prev, newRepo]);
    setSelectedRepo(newRepo.name);
    setActiveTab('overview');
  };

  // Theme Setter
  const handleSetTheme = (newTheme: 'light' | 'dark') => {
    setTheme(newTheme);
    if (newTheme === 'dark') {
      document.documentElement.classList.add('dark');
    } else {
      document.documentElement.classList.remove('dark');
    }
  };

  useEffect(() => {
    if (theme === 'dark') {
      document.documentElement.classList.add('dark');
    } else {
      document.documentElement.classList.remove('dark');
    }
  }, [theme]);

  useEffect(() => {
    checkServerOnline().then((online) => setIsServerOnline(online));

    Promise.all([
      fetchOverview().then(setOverview),
      fetchHealth().then(setHealth),
      fetchHotspots().then(setHotspots),
      fetchDecisions().then(setDecisions),
      fetchGraph().then(setGraph),
      fetchWikiPages().then(setWikiPages),
      fetchSettings().then(setSettings),
      fetchUsage().then(setUsage),
      fetchSymbols().then(setSymbols),
    ]);
  }, []);

  // Global Ctrl+K Shortcut for Search
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
        e.preventDefault();
        setIsSearchOpen(true);
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, []);

  useEffect(() => {
    if (searchQuery.trim().length > 1) {
      searchCodebase(searchQuery).then(setSearchResults);
    } else {
      setSearchResults([]);
    }
  }, [searchQuery]);

  return (
    <div className="min-h-screen bg-[#FAF8F5] dark:bg-[#0E1117] text-gray-900 dark:text-gray-100 flex font-sans transition-colors">
      {/* Sidebar Navigation */}
      <Sidebar
        activeTab={activeTab}
        onSelectTab={setActiveTab}
        theme={theme}
        onSetTheme={handleSetTheme}
        onOpenSearch={() => setIsSearchOpen(true)}
        workspaceRepos={customRepos}
        selectedRepo={selectedRepo}
        onSelectRepo={(r) => {
          setSelectedRepo(r);
          setActiveTab('overview');
        }}
        onOpenAddRepoModal={() => setIsAddRepoOpen(true)}
      />

      {/* Main Content Area */}
      <div className="flex-1 flex flex-col min-w-0">
        {/* Header Bar */}
        <header className="h-14 border-b border-[#EBE6DC] dark:border-[#2D333B] px-6 flex items-center justify-between sticky top-0 bg-[#FAF8F5]/90 dark:bg-[#0E1117]/90 backdrop-blur-md z-20">
          <div className="flex items-center space-x-2 text-xs font-mono text-gray-500">
            <span>Dashboard</span>
            <span>&gt;</span>
            {customRepos.length > 0 && selectedRepo && (
              <>
                <span className="text-gray-900 dark:text-white font-bold">{selectedRepo}</span>
                <span>&gt;</span>
              </>
            )}
            <span className="capitalize text-[#E67E22] font-semibold">{activeTab}</span>
          </div>

          <div className="flex items-center space-x-4">
            {/* Server Online Badge */}
            <div
              className={`px-3 py-1 rounded-full text-xs font-semibold flex items-center gap-1.5 border ${
                isServerOnline
                  ? 'bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border-emerald-500/20'
                  : 'bg-amber-500/10 text-amber-600 dark:text-amber-400 border-amber-500/20'
              }`}
            >
              {isServerOnline ? (
                <>
                  <CheckCircle2 size={12} /> Live Server (127.0.0.1:8080)
                </>
              ) : (
                <>
                  <AlertCircle size={12} /> Repowise Engine Ready
                </>
              )}
            </div>
          </div>
        </header>

        {/* Content Body */}
        <main className="flex-1 p-8 overflow-y-auto">
          {activeTab === 'overview' && (
            <OverviewTab
              overview={overview}
              health={health}
              hotspots={hotspots}
              onSelectTab={setActiveTab}
            />
          )}
          {activeTab === 'docs' && <DocsTab wikiPages={wikiPages} />}
          {activeTab === 'architecture' && (
            <ArchitectureTab graph={graph} symbols={symbols?.symbols || []} />
          )}
          {activeTab === 'graph' && <GraphTab graph={graph} />}
          {activeTab === 'health' && <HealthTab health={health} />}
          {activeTab === 'refactoring' && <HealthTab health={health} />}
          {activeTab === 'index' && <ArchitectureTab graph={graph} symbols={symbols?.symbols || []} />}
          {activeTab === 'commits' && <CommitsTab />}
          {activeTab === 'contributors' && <OverviewTab overview={overview} health={health} hotspots={hotspots} onSelectTab={setActiveTab} />}
          {activeTab === 'decisions' && <DecisionsTab decisions={decisions} />}
          {activeTab === 'chat' && <ChatTab />}
          {activeTab === 'usage' && <UsageTab usage={usage} />}
          {activeTab === 'workspace' && (
            <WorkspaceTab
              workspace={{ repos: customRepos }}
              onSelectRepo={(r) => {
                setSelectedRepo(r);
                setActiveTab('overview');
              }}
              onOpenAddRepoModal={() => setIsAddRepoOpen(true)}
              onClearWorkspace={() => {
                setCustomRepos([]);
              }}
            />
          )}
          {activeTab === 'settings' && <SettingsTab settings={settings} usage={usage} hasRepos={customRepos.length > 0} />}
        </main>
      </div>

      {/* GLOBAL SEARCH POPUP MODAL (CTRL+K) */}
      {isSearchOpen && (
        <div className="fixed inset-0 z-50 bg-black/50 backdrop-blur-sm flex items-start justify-center pt-20 p-4 animate-fade-in">
          <div className="repowise-card max-w-xl w-full rounded-2xl border shadow-2xl p-4 space-y-3 bg-white dark:bg-[#161B22]">
            <div className="relative flex items-center">
              <Search className="absolute left-3 text-gray-400" size={18} />
              <input
                type="text"
                autoFocus
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder="Search symbols or files (PageRank-biased)..."
                className="w-full bg-gray-100 dark:bg-[#0E1117] border border-gray-300 dark:border-gray-800 rounded-xl pl-10 pr-10 py-2.5 text-sm text-gray-900 dark:text-white focus:outline-none"
              />
              <button onClick={() => setIsSearchOpen(false)} className="absolute right-3 text-gray-400 hover:text-gray-600">
                <X size={18} />
              </button>
            </div>

            {searchResults.length > 0 && (
              <div className="space-y-1.5 max-h-80 overflow-y-auto pr-1 pt-2 border-t">
                {searchResults.map((res, idx) => (
                  <div
                    key={idx}
                    onClick={() => setIsSearchOpen(false)}
                    className="p-2.5 rounded-lg hover:bg-gray-100 dark:hover:bg-[#22272E] cursor-pointer flex items-center justify-between text-xs font-mono transition-colors"
                  >
                    <div>
                      <div className="font-bold text-[#E67E22]">{res.symbol_name || res.file}</div>
                      <div className="text-gray-500 truncate max-w-xs">{res.file}</div>
                    </div>
                    <span className="px-2 py-0.5 rounded text-[10px] uppercase font-bold bg-gray-200 dark:bg-gray-800 text-gray-700 dark:text-gray-300">
                      {res.kind || res.match_type}
                    </span>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      )}

      {/* ADD REPOSITORY MODAL */}
      <AddRepoModal
        isOpen={isAddRepoOpen}
        onClose={() => setIsAddRepoOpen(false)}
        onAddRepo={handleAddRepo}
      />
    </div>
  );
}

export default App;
