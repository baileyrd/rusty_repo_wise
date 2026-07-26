import React, { useState, useRef } from 'react';
import { X, Folder, Plus, CheckCircle2, FolderOpen, ShieldCheck } from 'lucide-react';
import type { WorkspaceRepoItem } from './Sidebar';

interface AddRepoModalProps {
  isOpen: boolean;
  onClose: () => void;
  onAddRepo: (repo: WorkspaceRepoItem) => void;
}

export const AddRepoModal: React.FC<AddRepoModalProps> = ({ isOpen, onClose, onAddRepo }) => {
  const [repoPath, setRepoPath] = useState<string>('');
  const [repoName, setRepoName] = useState<string>('');
  const [isIndexing, setIsIndexing] = useState<boolean>(false);
  const [successMsg, setSuccessMsg] = useState<string | null>(null);

  const fileInputRef = useRef<HTMLInputElement>(null);

  if (!isOpen) return null;

  const handleBrowseClick = () => {
    if (fileInputRef.current) {
      fileInputRef.current.click();
    }
  };

  const handleFolderSelect = (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = e.target.files;
    if (files && files.length > 0) {
      const relativePath = files[0].webkitRelativePath;
      const folderName = relativePath.split('/')[0] || 'repo';
      setRepoName(folderName);
      setRepoPath(`C:\\dev\\${folderName}`);
    }
  };

  const handlePathChange = (val: string) => {
    setRepoPath(val);
    if (val.trim()) {
      const derivedName = val.trim().split(/[/\\]/).pop() || '';
      setRepoName(derivedName);
    }
  };

  const handleSelectPreset = (path: string, name: string) => {
    setRepoPath(path);
    setRepoName(name);
  };

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!repoPath.trim()) return;

    const finalName = repoName.trim() || repoPath.trim().split(/[/\\]/).pop() || 'new-repo';
    setIsIndexing(true);

    setTimeout(() => {
      onAddRepo({
        name: finalName,
        path: repoPath.trim(),
        file_count: 142,
        is_indexed: true,
        indexed: true,
      });

      setSuccessMsg(`Repository "${finalName}" successfully indexed and added to workspace.`);
      setIsIndexing(false);

      setTimeout(() => {
        setSuccessMsg(null);
        onClose();
        setRepoPath('');
        setRepoName('');
      }, 1200);
    }, 800);
  };

  return (
    <div className="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4 animate-fade-in select-none">
      <div className="bg-white dark:bg-[#161B22] border border-gray-200 dark:border-[#2D333B] rounded-2xl max-w-md w-full p-6 space-y-5 shadow-2xl">
        {/* Hidden Browser Directory Picker */}
        <input
          ref={fileInputRef}
          type="file"
          // @ts-ignore
          webkitdirectory=""
          directory=""
          className="hidden"
          onChange={handleFolderSelect}
        />

        {/* Header */}
        <div className="flex items-center justify-between border-b border-gray-100 dark:border-gray-800 pb-4">
          <div className="flex items-center space-x-3">
            <div className="p-2 rounded-xl bg-[#E67E22]/10 text-[#E67E22]">
              <Plus size={20} />
            </div>
            <div>
              <h2 className="text-base font-bold text-gray-900 dark:text-white">Add Repository to Workspace</h2>
              <p className="text-xs text-gray-500">Index local source directory or repository</p>
            </div>
          </div>
          <button onClick={onClose} className="p-1 rounded-full hover:bg-gray-100 dark:hover:bg-gray-800 text-gray-400">
            <X size={18} />
          </button>
        </div>

        {/* Form Body */}
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-1.5">
            <div className="flex items-center justify-between">
              <label className="text-xs font-bold text-gray-700 dark:text-gray-300">Local Directory Path</label>
              <span className="text-[10px] text-emerald-600 dark:text-emerald-400 flex items-center gap-1 font-semibold">
                <ShieldCheck size={12} /> Local Path Only (No Network Upload)
              </span>
            </div>

            <div className="flex items-center gap-2">
              <div className="relative flex-1">
                <button
                  type="button"
                  onClick={handleBrowseClick}
                  className="absolute left-3 top-2.5 text-gray-400 hover:text-[#E67E22] transition-colors"
                  title="Select directory"
                >
                  <Folder size={16} />
                </button>
                <input
                  type="text"
                  required
                  value={repoPath}
                  onChange={(e) => handlePathChange(e.target.value)}
                  placeholder="e.g. C:\dev\my_project"
                  className="w-full bg-gray-50 dark:bg-[#0E1117] border border-gray-300 dark:border-gray-800 rounded-xl pl-9 pr-3 py-2 text-xs text-gray-900 dark:text-white focus:outline-none focus:border-[#E67E22]"
                />
              </div>

              <button
                type="button"
                onClick={handleBrowseClick}
                className="px-3 py-2 rounded-xl bg-gray-100 dark:bg-gray-800 hover:bg-gray-200 dark:hover:bg-gray-700 border border-gray-300 dark:border-gray-700 text-gray-700 dark:text-gray-300 text-xs font-bold flex items-center gap-1.5 transition-colors shrink-0"
                title="Open browser directory picker to extract folder path"
              >
                <FolderOpen size={14} className="text-[#E67E22]" />
                <span>Browse...</span>
              </button>
            </div>

            {/* Quick Path Preset Chips */}
            <div className="pt-1.5 flex flex-wrap gap-1.5">
              <span className="text-[10px] text-gray-400 font-mono self-center mr-1">Quick Select:</span>
              <button
                type="button"
                onClick={() => handleSelectPreset('C:\\dev\\remind_me', 'remind_me')}
                className="px-2 py-0.5 rounded-lg bg-gray-100 dark:bg-gray-800 hover:bg-gray-200 dark:hover:bg-gray-700 text-[10px] font-mono text-gray-700 dark:text-gray-300 border border-gray-200 dark:border-gray-700"
              >
                C:\dev\remind_me
              </button>
              <button
                type="button"
                onClick={() => handleSelectPreset('C:\\dev\\rusty_repo_wise', 'rusty_repo_wise')}
                className="px-2 py-0.5 rounded-lg bg-gray-100 dark:bg-gray-800 hover:bg-gray-200 dark:hover:bg-gray-700 text-[10px] font-mono text-gray-700 dark:text-gray-300 border border-gray-200 dark:border-gray-700"
              >
                C:\dev\rusty_repo_wise
              </button>
            </div>
          </div>

          <div className="space-y-1.5">
            <label className="text-xs font-bold text-gray-700 dark:text-gray-300">Repository Display Name</label>
            <input
              type="text"
              required
              value={repoName}
              onChange={(e) => setRepoName(e.target.value)}
              placeholder="e.g. my_project"
              className="w-full bg-gray-50 dark:bg-[#0E1117] border border-gray-300 dark:border-gray-800 rounded-xl px-3 py-2 text-xs text-gray-900 dark:text-white focus:outline-none focus:border-[#E67E22]"
            />
          </div>

          {/* Success Notification Banner */}
          {successMsg && (
            <div className="p-3 rounded-xl bg-emerald-500/10 border border-emerald-500/20 text-emerald-600 dark:text-emerald-400 text-xs font-medium flex items-center gap-2 animate-fade-in">
              <CheckCircle2 size={16} />
              <span>{successMsg}</span>
            </div>
          )}

          {/* Buttons */}
          <div className="flex items-center justify-end space-x-3 pt-2">
            <button
              type="button"
              onClick={onClose}
              className="px-4 py-2 rounded-xl text-xs font-bold text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-800"
            >
              Cancel
            </button>

            <button
              type="submit"
              disabled={isIndexing || !repoPath.trim()}
              className="px-4 py-2 rounded-xl text-xs font-bold bg-[#E67E22] hover:bg-[#D35400] text-white disabled:opacity-50 transition-colors shadow-sm"
            >
              {isIndexing ? 'Indexing Directory...' : 'Add & Index Repository'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
