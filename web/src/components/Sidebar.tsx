import React, { useState } from 'react';
import {
  LayoutGrid,
  Settings,
  Search,
  Activity,
  BookOpen,
  GitFork,
  Network,
  Heart,
  Wrench,
  Files,
  GitCommit,
  Users,
  Lightbulb,
  MessageSquare,
  BarChart3,
  DollarSign,
  ChevronDown,
  ChevronRight,
  Sun,
  Moon,
  Plus,
  PanelLeftClose,
  Folder,
  Layers,
} from 'lucide-react';

export interface WorkspaceRepoItem {
  name: string;
  file_count: number;
  is_indexed: boolean;
  indexed: boolean;
  path?: string;
}

interface SidebarProps {
  activeTab: string;
  onSelectTab: (tabId: string) => void;
  theme: 'light' | 'dark';
  onSetTheme: (theme: 'light' | 'dark') => void;
  onOpenSearch: () => void;
  workspaceRepos?: WorkspaceRepoItem[];
  selectedRepo?: string;
  onSelectRepo?: (repoName: string) => void;
  onOpenAddRepoModal?: () => void;
}

export const Sidebar: React.FC<SidebarProps> = ({
  activeTab,
  onSelectTab,
  theme,
  onSetTheme,
  onOpenSearch,
  workspaceRepos = [],
  selectedRepo = '',
  onSelectRepo,
  onOpenAddRepoModal,
}) => {
  const [repoOpen, setRepoOpen] = useState<boolean>(true);
  const [workspaceOpen, setWorkspaceOpen] = useState<boolean>(true);

  const mainNav = [
    { id: 'overview', label: 'Dashboard', icon: LayoutGrid },
    { id: 'workspace', label: 'Multi-Repo Workspace', icon: Layers },
    { id: 'settings', label: 'Settings', icon: Settings },
  ];

  const repoNav = [
    { id: 'overview', label: 'Overview', icon: Activity },
    { id: 'docs', label: 'Docs', icon: BookOpen },
    { id: 'architecture', label: 'Architecture', icon: GitFork },
    { id: 'graph', label: 'Knowledge Graph', icon: Network },
    { id: 'health', label: 'Code Health', icon: Heart },
    { id: 'refactoring', label: 'Refactoring', icon: Wrench },
    { id: 'index', label: 'Files', icon: Files },
  ];

  const peopleNav = [
    { id: 'commits', label: 'Commits', icon: GitCommit },
    { id: 'contributors', label: 'Contributors', icon: Users },
    { id: 'decisions', label: 'Decisions', icon: Lightbulb },
  ];

  const settingsNav = [
    { id: 'stats', label: 'Stats', icon: BarChart3 },
    { id: 'usage', label: 'Usage & savings', icon: DollarSign },
    { id: 'settings', label: 'Settings', icon: Settings },
  ];

  const handleRepoClick = (repoName: string) => {
    if (onSelectRepo) {
      onSelectRepo(repoName);
    }
  };

  const hasRepos = workspaceRepos.length > 0;
  const currentSelectedRepo = selectedRepo || (hasRepos ? workspaceRepos[0].name : '');

  return (
    <aside className="w-64 bg-white dark:bg-[#161B22] border-r border-gray-200 dark:border-[#2D333B] flex flex-col h-screen sticky top-0 text-xs font-sans select-none z-30 transition-colors">
      {/* Brand Header */}
      <div className="p-4 flex items-center justify-between">
        <div className="flex items-center space-x-2 cursor-pointer" onClick={() => onSelectTab('overview')}>
          <svg className="w-6 h-6 text-[#9A6614] dark:text-[#E67E22]" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-1 15h-2v-2h2v2zm0-4h-2V7h2v6zm4 4h-2v-2h2v2zm0-4h-2V7h2v6z" />
          </svg>
          <span className="font-extrabold text-base tracking-tight text-gray-900 dark:text-white">repowise</span>
        </div>

        <button className="text-gray-400 hover:text-gray-700 dark:hover:text-gray-200 p-1">
          <PanelLeftClose size={18} />
        </button>
      </div>

      {/* Top Quick Links */}
      <div className="px-3 pb-3 space-y-1 border-b border-gray-100 dark:border-[#2D333B]">
        {mainNav.map((item) => {
          const Icon = item.icon;
          const isActive = activeTab === item.id;
          return (
            <button
              key={item.id}
              onClick={() => onSelectTab(item.id)}
              className={`w-full text-left px-3 py-2 rounded-lg font-medium flex items-center justify-between transition-colors ${
                isActive
                  ? 'bg-gray-100 dark:bg-[#22272E] text-gray-900 dark:text-white font-bold'
                  : 'text-gray-600 dark:text-gray-400 hover:bg-gray-50 dark:hover:bg-[#22272E]/50'
              }`}
            >
              <div className="flex items-center space-x-3">
                <Icon size={16} className="text-gray-500" />
                <span className="text-xs">{item.label}</span>
              </div>
            </button>
          );
        })}

        <button
          onClick={onOpenSearch}
          className="w-full text-left px-3 py-2 rounded-lg font-medium flex items-center justify-between text-gray-600 dark:text-gray-400 hover:bg-gray-50 dark:hover:bg-[#22272E]/50 transition-colors"
        >
          <div className="flex items-center space-x-3">
            <Search size={16} className="text-gray-500" />
            <span className="text-xs">Search</span>
          </div>
          <kbd className="px-1.5 py-0.5 rounded text-[10px] bg-gray-100 dark:bg-gray-800 text-gray-400 font-mono border border-gray-200 dark:border-gray-700">⌘K</kbd>
        </button>
      </div>

      {/* Main Navigation Scroll Area */}
      <div className="flex-1 overflow-y-auto px-3 py-3 space-y-4">
        {/* MULTI-REPOSITORY WORKSPACE SECTION */}
        <div>
          <button
            onClick={() => setWorkspaceOpen(!workspaceOpen)}
            className="w-full text-left text-[10px] font-bold tracking-wider uppercase text-gray-400 dark:text-gray-500 flex items-center justify-between px-1 mb-1"
          >
            <span>WORKSPACE ({workspaceRepos.length} REPOS)</span>
            <ChevronRight size={12} className={workspaceOpen ? 'rotate-90 transition-transform' : ''} />
          </button>

          {workspaceOpen && (
            <div className="space-y-1 pl-1 pt-1">
              {workspaceRepos.length === 0 ? (
                <div className="text-[11px] text-gray-400 px-2 py-1 font-mono italic">
                  No repos in workspace
                </div>
              ) : (
                workspaceRepos.map((repo) => (
                  <button
                    key={repo.name}
                    onClick={() => handleRepoClick(repo.name)}
                    className={`w-full text-left px-2 py-1.5 rounded-lg flex items-center justify-between text-xs font-medium transition-colors ${
                      currentSelectedRepo === repo.name
                        ? 'bg-[#FDF3E7] dark:bg-[#9A6614]/20 text-[#9A6614] dark:text-[#E67E22] font-bold'
                        : 'text-gray-600 dark:text-gray-400 hover:bg-gray-50 dark:hover:bg-[#22272E]/50'
                    }`}
                  >
                    <div className="flex items-center space-x-2 truncate">
                      <Folder size={14} className={currentSelectedRepo === repo.name ? 'text-[#9A6614] dark:text-[#E67E22]' : 'text-gray-400'} />
                      <span className="truncate">{repo.name}</span>
                    </div>
                    <span className="text-[10px] font-mono opacity-70">{repo.file_count}</span>
                  </button>
                ))
              )}
            </div>
          )}
        </div>

        {/* REPOSITORIES SECTION */}
        <div>
          <div className="text-[10px] font-bold tracking-wider uppercase text-gray-400 dark:text-gray-500 px-1 mb-2">
            REPOSITORIES
          </div>

          {!hasRepos ? (
            /* BLANK STATE IN SIDEBAR WHEN ZERO REPOS LOADED */
            <div className="p-3 rounded-xl border border-dashed border-gray-200 dark:border-gray-800 text-center space-y-2 my-2">
              <div className="text-xs text-gray-400 font-medium">No repositories loaded</div>
              <button
                onClick={onOpenAddRepoModal}
                className="w-full px-2 py-1.5 rounded-lg bg-[#E67E22] hover:bg-[#D35400] text-white text-xs font-bold transition-colors inline-flex items-center justify-center gap-1 shadow-sm"
              >
                <Plus size={13} /> Add Repository
              </button>
            </div>
          ) : (
            <div className="space-y-1">
              {/* Active Selected Repo */}
              <button
                onClick={() => setRepoOpen(!repoOpen)}
                className="w-full text-left px-2 py-1 flex items-center justify-between text-xs font-bold text-[#9A6614] dark:text-[#E67E22]"
              >
                <div className="flex items-center space-x-2">
                  <span className="w-2 h-2 rounded-full bg-[#9A6614] dark:bg-[#E67E22]" />
                  <span>{currentSelectedRepo}</span>
                </div>
                <ChevronDown size={12} className={repoOpen ? '' : '-rotate-90 transition-transform'} />
              </button>

              {repoOpen && (
                <div className="pl-4 space-y-0.5 border-l border-gray-100 dark:border-gray-800 ml-3">
                  {repoNav.map((item) => {
                    const Icon = item.icon;
                    const isActive = activeTab === item.id;
                    const isHealth = item.id === 'health';
                    return (
                      <button
                        key={item.id}
                        onClick={() => onSelectTab(item.id)}
                        className={`w-full text-left px-3 py-1.5 rounded-xl font-medium flex items-center space-x-2.5 transition-all ${
                          isActive
                            ? isHealth
                              ? 'bg-[#FDF3E7] dark:bg-[#9A6614]/20 text-[#9A6614] dark:text-[#E67E22] font-bold shadow-sm'
                              : 'bg-gray-100 dark:bg-[#22272E] text-gray-900 dark:text-white font-bold shadow-sm'
                            : 'text-gray-600 dark:text-gray-400 hover:bg-gray-50 dark:hover:bg-[#22272E]/50'
                        }`}
                      >
                        <Icon size={15} className={isActive ? (isHealth ? 'text-[#9A6614] dark:text-[#E67E22]' : 'text-gray-900 dark:text-white') : 'text-gray-400'} />
                        <span className="text-xs">{item.label}</span>
                      </button>
                    );
                  })}

                  {/* PEOPLE & HISTORY */}
                  <div className="pt-3 pb-1">
                    <div className="text-[10px] font-bold uppercase tracking-wider text-gray-400 px-2 mb-1.5">
                      PEOPLE & HISTORY
                    </div>
                    <div className="space-y-0.5">
                      {peopleNav.map((item) => {
                        const Icon = item.icon;
                        const isActive = activeTab === item.id;
                        return (
                          <button
                            key={item.id}
                            onClick={() => onSelectTab(item.id)}
                            className={`w-full text-left px-3 py-1.5 rounded-xl font-medium flex items-center space-x-2.5 transition-all ${
                              isActive
                                ? 'bg-gray-100 dark:bg-[#22272E] text-gray-900 dark:text-white font-bold shadow-sm'
                                : 'text-gray-600 dark:text-gray-400 hover:bg-gray-50 dark:hover:bg-[#22272E]/50'
                            }`}
                          >
                            <Icon size={15} className={isActive ? 'text-gray-900 dark:text-white' : 'text-gray-400'} />
                            <span className="text-xs">{item.label}</span>
                          </button>
                        );
                      })}

                      <button
                        onClick={() => onSelectTab('chat')}
                        className={`w-full text-left px-3 py-1.5 rounded-xl font-medium flex items-center space-x-2.5 transition-all ${
                          activeTab === 'chat'
                            ? 'bg-gray-100 dark:bg-[#22272E] text-gray-900 dark:text-white font-bold shadow-sm'
                            : 'text-gray-600 dark:text-gray-400 hover:bg-gray-50 dark:hover:bg-[#22272E]/50'
                        }`}
                      >
                        <MessageSquare size={15} className={activeTab === 'chat' ? 'text-gray-900 dark:text-white' : 'text-gray-400'} />
                        <span className="text-xs">Chat</span>
                      </button>
                    </div>
                  </div>

                  {/* SETTINGS SECTION */}
                  <div className="pt-2 pb-1">
                    <div className="text-[10px] font-bold uppercase tracking-wider text-gray-400 px-2 mb-1.5">
                      SETTINGS
                    </div>
                    <div className="space-y-0.5">
                      {settingsNav.map((item) => {
                        const Icon = item.icon;
                        const isActive = activeTab === item.id;
                        return (
                          <button
                            key={item.id}
                            onClick={() => onSelectTab(item.id)}
                            className={`w-full text-left px-3 py-1.5 rounded-xl font-medium flex items-center space-x-2.5 transition-all ${
                              isActive
                                ? 'bg-gray-100 dark:bg-[#22272E] text-gray-900 dark:text-white font-bold shadow-sm'
                                : 'text-gray-600 dark:text-gray-400 hover:bg-gray-50 dark:hover:bg-[#22272E]/50'
                            }`}
                          >
                            <Icon size={15} className={isActive ? 'text-gray-900 dark:text-white' : 'text-gray-400'} />
                            <span className="text-xs">{item.label}</span>
                          </button>
                        );
                      })}
                    </div>
                  </div>
                </div>
              )}

              {/* Other Repositories */}
              {workspaceRepos
                .filter((r) => r.name !== currentSelectedRepo)
                .map((r) => (
                  <div
                    key={r.name}
                    onClick={() => handleRepoClick(r.name)}
                    className="px-2 py-1 flex items-center justify-between text-xs font-medium text-gray-600 dark:text-gray-400 cursor-pointer hover:text-gray-900"
                  >
                    <div className="flex items-center space-x-2">
                      <span className="w-2 h-2 rounded-full bg-gray-400" />
                      <span>{r.name}</span>
                    </div>
                    <ChevronRight size={12} />
                  </div>
                ))}

              <button
                onClick={onOpenAddRepoModal}
                className="w-full text-left px-2 py-1.5 text-xs font-medium text-gray-500 hover:text-gray-800 dark:hover:text-white flex items-center space-x-2 transition-colors pt-2"
              >
                <Plus size={14} />
                <span>Add Repository</span>
              </button>
            </div>
          )}
        </div>
      </div>

      {/* Footer Controls */}
      <div className="p-3 border-t border-gray-200 dark:border-[#2D333B] space-y-3">
        {/* Light / Dark Theme Pill Switcher Container */}
        <div className="p-1 rounded-2xl bg-[#F4EAE1] dark:bg-gray-800/80 flex items-center justify-between border border-gray-200/80 dark:border-gray-700">
          <button
            onClick={() => onSetTheme('light')}
            className={`w-1/2 py-1.5 rounded-xl text-xs font-extrabold flex items-center justify-center space-x-1.5 transition-all ${
              theme === 'light'
                ? 'bg-white text-gray-900 shadow-sm'
                : 'text-gray-500 hover:text-gray-900 dark:text-gray-400 dark:hover:text-white'
            }`}
          >
            <Sun size={14} className={theme === 'light' ? 'text-amber-500' : 'text-gray-400'} />
            <span>Light</span>
          </button>

          <button
            onClick={() => onSetTheme('dark')}
            className={`w-1/2 py-1.5 rounded-xl text-xs font-extrabold flex items-center justify-center space-x-1.5 transition-all ${
              theme === 'dark'
                ? 'bg-[#1C2128] text-white shadow-sm'
                : 'text-gray-500 hover:text-gray-900 dark:text-gray-400 dark:hover:text-white'
            }`}
          >
            <Moon size={14} className={theme === 'dark' ? 'text-blue-400' : 'text-gray-400'} />
            <span>Dark</span>
          </button>
        </div>

        {/* Send feedback link */}
        <button className="w-full text-left px-1 text-[11px] font-medium text-gray-500 hover:text-gray-800 dark:hover:text-white flex items-center space-x-2">
          <MessageSquare size={13} />
          <span>Send feedback</span>
        </button>
      </div>
    </aside>
  );
};
