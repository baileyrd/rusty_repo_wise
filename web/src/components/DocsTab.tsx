import React, { useState } from 'react';
import type { WikiPageInfo } from '../types/api';
import { Search, Download, Play, ChevronRight, X, ArrowLeft, ArrowRight } from 'lucide-react';

interface DocsTabProps {
  wikiPages: WikiPageInfo[];
}

export const DocsTab: React.FC<DocsTabProps> = ({ wikiPages: _wikiPages }) => {
  const [filterMode, setFilterMode] = useState<'domain' | 'folder'>('domain');
  const [activeDoc, setActiveDoc] = useState<string>('Project Overview');
  const [isPresenting, setIsPresenting] = useState<boolean>(false);
  const [slideIndex, setSlideIndex] = useState<number>(0);

  const guidedTourItems = [
    { id: 'Project Overview', label: 'Project Overview' },
    { id: 'Architecture Guide', label: 'Architecture Guide' },
    { id: 'Guided Tour', label: 'Guided Tour' },
    { id: 'Getting Started', label: 'Getting Started' },
    { id: 'Codebase Map', label: 'Codebase Map' },
    { id: 'Key Concepts', label: 'Key Concepts' },
    { id: 'How It Works', label: 'How It Works' },
    { id: 'Active Landscape', label: 'Active Landscape' },
  ];

  const architectureLayers = [
    'Automated Test Suite',
    'Application',
    'API',
    'CLI',
    'Config',
    'Documentation Toolchain',
    'Domain Type Definitions',
    'Ingestion and Analysis Engine',
    'Persistence and Storage',
    'Shared UI Helpers',
    'UI',
  ];

  const presentationSlides = [
    { title: 'repowise', subtitle: 'Repository Overview & Knowledge Engine', body: 'Repowise is a codebase documentation engine that ingests source code and metadata, transforms them through an ingestion + analysis pipeline, and outputs a searchable documentation knowledge base.' },
    { title: 'Layer: Automated Test Suite', subtitle: 'System & Integration Test Suite', body: 'Validates system end-to-end by orchestrating unit and integration tests across API parser, persistence layer, and UI webviews.' },
    { title: 'Layer: CLI', subtitle: 'Command-Line Interface & Orchestration', body: 'Provides command-line interface tools for running ingestion/analysis workflows, managing search vector storage, and launching servers.' },
    { title: 'Layer: Persistence and Search', subtitle: 'Storage Engine & Vector Indexing', body: 'Handles database CRUD, ORM model migrations, and hybrid text + vector search storage.' },
  ];

  return (
    <div className="space-y-4 animate-fade-in max-w-7xl mx-auto select-none">
      {/* Top Action Bar */}
      <div className="repowise-card p-4 rounded-xl border flex flex-col md:flex-row md:items-center justify-between gap-4">
        <div className="flex items-center space-x-3">
          <div className="flex items-center bg-gray-200 dark:bg-gray-800 p-0.5 rounded-lg border border-gray-300 dark:border-gray-700">
            <button
              onClick={() => setFilterMode('domain')}
              className={`px-3 py-1 rounded text-xs font-semibold ${
                filterMode === 'domain' ? 'bg-white dark:bg-[#1C2128] text-[#E67E22] shadow-sm' : 'text-gray-500'
              }`}
            >
              By domain
            </button>
            <button
              onClick={() => setFilterMode('folder')}
              className={`px-3 py-1 rounded text-xs font-semibold ${
                filterMode === 'folder' ? 'bg-white dark:bg-[#1C2128] text-[#E67E22] shadow-sm' : 'text-gray-500'
              }`}
            >
              By folder
            </button>
          </div>
          <span className="px-2.5 py-0.5 rounded-full text-xs font-bold bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border border-emerald-500/20 flex items-center gap-1">
            <span className="w-1.5 h-1.5 rounded-full bg-emerald-500" /> Fresh
          </span>
        </div>

        <div className="flex items-center space-x-2">
          <button className="px-3 py-1.5 rounded-lg border text-xs font-medium flex items-center space-x-1.5 text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-800">
            <Search size={14} /> <span>Search</span> <kbd className="text-[10px] opacity-60">⌘K</kbd>
          </button>
          <button className="px-3 py-1.5 rounded-lg border text-xs font-medium flex items-center space-x-1.5 text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-800">
            <Download size={14} /> <span>Export</span>
          </button>
          <button
            onClick={() => setIsPresenting(true)}
            className="px-3.5 py-1.5 rounded-lg bg-[#E67E22] hover:bg-[#D35400] text-white text-xs font-bold flex items-center space-x-1.5 shadow-sm transition-colors"
          >
            <Play size={14} /> <span>Present</span>
          </button>
        </div>
      </div>

      {/* Main Docs Content Layout */}
      <div className="grid grid-cols-1 lg:grid-cols-4 gap-6">
        {/* Left Sidebar Guide Tree */}
        <div className="repowise-card p-4 rounded-xl border space-y-4 max-h-[750px] overflow-y-auto">
          <div>
            <div className="text-[11px] font-bold uppercase tracking-wider text-gray-400 mb-2">GUIDED TOUR</div>
            <div className="space-y-0.5">
              {guidedTourItems.map((item) => (
                <button
                  key={item.id}
                  onClick={() => setActiveDoc(item.id)}
                  className={`w-full text-left px-2.5 py-1.5 rounded-md text-xs font-medium transition-colors flex items-center justify-between ${
                    activeDoc === item.id
                      ? 'bg-[#E67E22]/15 text-[#E67E22] font-bold'
                      : 'text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-800/60'
                  }`}
                >
                  <span>{item.label}</span>
                  {activeDoc === item.id && <ChevronRight size={12} />}
                </button>
              ))}
            </div>
          </div>

          <div className="pt-3 border-t border-gray-200 dark:border-gray-800">
            <div className="text-[11px] font-bold uppercase tracking-wider text-gray-400 mb-2">ARCHITECTURE</div>
            <div className="space-y-0.5">
              <div className="text-xs font-semibold text-gray-500 px-2 mb-1 flex items-center justify-between">
                <span>Layers (11)</span>
              </div>
              {architectureLayers.map((layer) => (
                <button
                  key={layer}
                  onClick={() => setActiveDoc(layer)}
                  className={`w-full text-left px-2.5 py-1 rounded-md text-[11px] font-medium transition-colors ${
                    activeDoc === layer
                      ? 'bg-[#E67E22]/15 text-[#E67E22] font-bold'
                      : 'text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-800/60'
                  }`}
                >
                  {layer}
                </button>
              ))}
            </div>
          </div>
        </div>

        {/* Main Document Panel */}
        <div className="repowise-card p-8 rounded-xl border lg:col-span-3 space-y-6">
          <div className="border-b border-gray-200 dark:border-gray-800 pb-4">
            <div className="text-xs text-gray-400 font-mono uppercase tracking-wider">repowise</div>
            <h1 className="text-2xl font-black text-gray-900 dark:text-white mt-1">Repository Overview: repowise</h1>
          </div>

          <div className="space-y-6 text-sm text-gray-700 dark:text-gray-300 leading-relaxed font-sans">
            <section className="space-y-2">
              <h2 className="text-lg font-bold text-gray-900 dark:text-white">Project Summary</h2>
              <p>
                Repowise is a repository documentation engine that ingests source code and metadata (file contents,
                language specs, git context, and dependency signals), transforms them through an ingestion + analysis
                pipeline (parsing/resolution, dependency and coupling analysis, health/risk metrics, and decision/wiki synthesis),
                and outputs a searchable documentation knowledge base with generated wiki pages.
              </p>
            </section>

            <section className="space-y-3 pt-2 border-t border-gray-200 dark:border-gray-800">
              <h2 className="text-lg font-bold text-gray-900 dark:text-white">Technology Stack</h2>
              <ul className="space-y-2 pl-4 list-disc">
                <li>
                  <strong className="text-gray-900 dark:text-white">Python (core ingestion + analysis + server):</strong>
                  <ul className="pl-4 mt-1 space-y-1 text-xs font-mono text-gray-600 dark:text-gray-400">
                    <li><code className="text-[#E67E22]">packages/core</code> &mdash; ingestion, resolvers, language specs, analysis models</li>
                    <li><code className="text-[#E67E22]">packages/server</code> &mdash; application server and MCP tooling endpoints</li>
                    <li><code className="text-[#E67E22]">packages/cli</code> &mdash; command-line entry for running workflows</li>
                  </ul>
                </li>
                <li>
                  <strong className="text-gray-900 dark:text-white">TypeScript (client/UI + shared types):</strong>
                  <ul className="pl-4 mt-1 space-y-1 text-xs font-mono text-gray-600 dark:text-gray-400">
                    <li><code className="text-[#E67E22]">packages/types</code> &mdash; shared type definitions for API contracts</li>
                    <li><code className="text-[#E67E22]">packages/web</code> &mdash; web application frontend</li>
                  </ul>
                </li>
              </ul>
            </section>

            {/* Interactive Architecture Flow Diagram Box */}
            <section className="space-y-3 pt-4 border-t border-gray-200 dark:border-gray-800">
              <h3 className="text-base font-bold text-gray-900 dark:text-white">Key Dependency Flows (Diagram)</h3>
              <div className="p-6 rounded-xl bg-gray-50 dark:bg-[#0E1117] border border-gray-200 dark:border-gray-800 text-center space-y-4">
                <div className="inline-block px-4 py-1.5 rounded-lg bg-blue-500/10 text-blue-600 dark:text-blue-400 border border-blue-500/20 font-mono text-xs font-bold">
                  Shared UI Utilities
                </div>
                <div className="flex justify-center space-x-6">
                  <div className="px-4 py-2 rounded-lg bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border border-emerald-500/20 font-mono text-xs font-bold">
                    Persistence and Search
                  </div>
                  <div className="px-4 py-2 rounded-lg bg-amber-500/10 text-amber-600 dark:text-amber-400 border border-amber-500/20 font-mono text-xs font-bold">
                    Ingestion and Reasoning
                  </div>
                </div>
              </div>
            </section>
          </div>
        </div>
      </div>

      {/* PRESENTATION DECK MODAL OVERLAY */}
      {isPresenting && (
        <div className="fixed inset-0 z-50 bg-[#FAF8F5] dark:bg-[#0E1117] flex flex-col justify-between p-12 animate-fade-in">
          {/* Deck Header */}
          <div className="flex items-center justify-between border-b border-gray-300 dark:border-gray-800 pb-4">
            <div className="flex items-center space-x-2">
              <span className="font-extrabold text-[#E67E22] text-xl">repowise</span>
              <span className="text-xs text-gray-400 font-mono">/ Deck Mode</span>
            </div>
            <button
              onClick={() => setIsPresenting(false)}
              className="p-2 rounded-full hover:bg-gray-200 dark:hover:bg-gray-800 text-gray-600 dark:text-gray-300"
            >
              <X size={24} />
            </button>
          </div>

          {/* Slide Body */}
          <div className="max-w-4xl mx-auto text-center space-y-6 my-auto">
            <div className="text-xs font-mono font-bold uppercase tracking-wider text-[#E67E22]">
              Slide {slideIndex + 1} of {presentationSlides.length}
            </div>
            <h1 className="text-4xl font-black text-gray-900 dark:text-white">
              {presentationSlides[slideIndex].title}
            </h1>
            <div className="text-lg font-semibold text-gray-600 dark:text-gray-400">
              {presentationSlides[slideIndex].subtitle}
            </div>
            <p className="text-base text-gray-700 dark:text-gray-300 max-w-2xl mx-auto leading-relaxed">
              {presentationSlides[slideIndex].body}
            </p>
          </div>

          {/* Deck Footer Controls */}
          <div className="flex items-center justify-between border-t border-gray-300 dark:border-gray-800 pt-4">
            <div className="text-xs font-mono text-gray-400">Use Arrow Keys to Navigate</div>
            <div className="flex items-center space-x-4">
              <button
                disabled={slideIndex === 0}
                onClick={() => setSlideIndex((i) => Math.max(i - 1, 0))}
                className="p-3 rounded-lg border hover:bg-gray-200 dark:hover:bg-gray-800 disabled:opacity-30"
              >
                <ArrowLeft size={18} />
              </button>
              <span className="font-mono text-sm font-bold">{slideIndex + 1} / {presentationSlides.length}</span>
              <button
                disabled={slideIndex === presentationSlides.length - 1}
                onClick={() => setSlideIndex((i) => Math.min(i + 1, presentationSlides.length - 1))}
                className="p-3 rounded-lg border hover:bg-gray-200 dark:hover:bg-gray-800 disabled:opacity-30"
              >
                <ArrowRight size={18} />
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};
