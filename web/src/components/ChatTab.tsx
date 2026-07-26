import React, { useState } from 'react';
import type { ChatMessage } from '../types/api';
import { sendChatMessage } from '../services/api';
import { Sparkles, Send, ChevronDown, ChevronRight, FileText } from 'lucide-react';

export const ChatTab: React.FC = () => {
  const [messages, setMessages] = useState<ChatMessage[]>([
    {
      id: '1',
      sender: 'assistant',
      text: 'Here are the highest risk-score files to modify (top hotspots risk score / detect profile / radius, based on get_risk):\n\n1. `packages/core/src/repowise/persistence/models.py` — **hotspot_score = 0.7773** (150 dependents, bug-prone feature, active, 15 days ago, owned ~90% by Raghav Chamaliya)\n2. `packages/server/src/repowise/server/app.py` — **hotspot_score = 0.6542** (100% churn-heavy, feature-author, content ~86% by Swat Ahuja)\n3. `packages/codebase/src/repowise/analysis/coupling.py` — **hotspot_score = 0.6050** (100% churn-heavy, owned 100% by Sawat Ahuja)',
      timestamp: 'Just now',
      thinking_steps: ['Querying get_risk index (2 steps)', 'Filtering top 5 churn x complexity hotspots'],
      citations: [
        'packages/core/src/repowise/persistence/models.py',
        'packages/server/src/repowise/server/app.py',
        'packages/codebase/src/repowise/analysis/coupling.py',
      ],
    },
  ]);

  const [inputPrompt, setInputPrompt] = useState<string>('');
  const [loading, setLoading] = useState<boolean>(false);
  const [openThinking, setOpenThinking] = useState<Record<string, boolean>>({ '1': true });

  const shortcutPrompts = [
    'Give me an overview of this codebase',
    'What are the highest risk files to modify?',
    'Show me the architecture diagram',
    'What dead code can be safely removed?',
    'What architectural decisions have been made?',
  ];

  const handleSend = async (promptText?: string) => {
    const textToSend = promptText || inputPrompt;
    if (!textToSend.trim()) return;

    const userMsg: ChatMessage = {
      id: Date.now().toString(),
      sender: 'user',
      text: textToSend,
      timestamp: 'Just now',
    };

    setMessages((prev) => [...prev, userMsg]);
    setInputPrompt('');
    setLoading(true);

    try {
      const res = await sendChatMessage(textToSend);
      const botMsg: ChatMessage = {
        id: (Date.now() + 1).toString(),
        sender: 'assistant',
        text: res.reply,
        timestamp: 'Just now',
        thinking_steps: res.thinking_steps || ['Ingesting workspace index', 'Querying graph dependency paths'],
        citations: res.citations || [],
      };
      setMessages((prev) => [...prev, botMsg]);
    } catch {
      // Fallback
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="space-y-6 animate-fade-in max-w-5xl mx-auto select-none">
      {/* Header */}
      <div className="repowise-card p-6 rounded-xl border flex items-center justify-between">
        <div>
          <h2 className="text-xl font-bold text-gray-900 dark:text-white flex items-center gap-2">
            <Sparkles className="text-[#E67E22]" size={24} />
            Ask anything about repowise
          </h2>
          <p className="text-xs text-gray-500 mt-1">
            Explore architecture, risk score, code health, issue dependencies, and understand decisions.
          </p>
        </div>

        <div className="px-3 py-1 rounded-full text-xs font-mono font-bold bg-[#E67E22]/10 text-[#E67E22] border border-[#E67E22]/20">
          OpenAI gpt-5.4-nano
        </div>
      </div>

      {/* Suggested Prompts Cards */}
      {messages.length === 0 && (
        <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
          {shortcutPrompts.map((p, idx) => (
            <button
              key={idx}
              onClick={() => handleSend(p)}
              className="p-4 rounded-xl border bg-white dark:bg-[#1C2128] text-left hover:border-[#E67E22] text-xs font-medium text-gray-800 dark:text-gray-200 transition-all shadow-sm"
            >
              {p}
            </button>
          ))}
        </div>
      )}

      {/* Messages Thread */}
      <div className="space-y-4">
        {messages.map((m) => (
          <div key={m.id} className="space-y-2">
            {m.sender === 'user' ? (
              <div className="flex justify-end">
                <div className="max-w-xl p-4 rounded-2xl bg-[#E67E22] text-white text-xs font-medium shadow-sm">
                  {m.text}
                </div>
              </div>
            ) : (
              <div className="repowise-card p-6 rounded-2xl border space-y-4">
                {/* Collapsible Thinking Process */}
                {m.thinking_steps && m.thinking_steps.length > 0 && (
                  <div className="border-b border-gray-200 dark:border-gray-800 pb-3">
                    <button
                      onClick={() => setOpenThinking((prev) => ({ ...prev, [m.id]: !prev[m.id] }))}
                      className="text-xs font-mono text-gray-500 flex items-center space-x-1.5 hover:text-gray-900 dark:hover:text-white"
                    >
                      <Sparkles size={14} className="text-[#E67E22]" />
                      <span>Thinking &bull; {m.thinking_steps.length} steps</span>
                      {openThinking[m.id] ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
                    </button>

                    {openThinking[m.id] && (
                      <div className="mt-2 pl-4 border-l border-gray-300 dark:border-gray-700 space-y-1 text-xs font-mono text-gray-500">
                        {m.thinking_steps.map((step, idx) => (
                          <div key={idx}>&bull; {step}</div>
                        ))}
                      </div>
                    )}
                  </div>
                )}

                {/* Main Response Text */}
                <div className="text-xs text-gray-800 dark:text-gray-200 font-sans leading-relaxed whitespace-pre-line">
                  {m.text}
                </div>

                {/* Citations */}
                {m.citations && m.citations.length > 0 && (
                  <div className="pt-3 border-t border-gray-200 dark:border-gray-800 space-y-1.5">
                    <div className="text-[10px] font-bold text-gray-400 uppercase tracking-wider">Sources & Citations</div>
                    <div className="flex flex-wrap gap-2">
                      {m.citations.map((c, idx) => (
                        <span
                          key={idx}
                          className="px-2.5 py-1 rounded bg-gray-100 dark:bg-[#0E1117] border border-gray-300 dark:border-gray-800 font-mono text-[10px] text-blue-500 flex items-center gap-1"
                        >
                          <FileText size={12} /> {c}
                        </span>
                      ))}
                    </div>
                  </div>
                )}
              </div>
            )}
          </div>
        ))}

        {loading && (
          <div className="p-4 rounded-xl border bg-white dark:bg-[#1C2128] text-xs font-mono text-gray-500 animate-pulse">
            Analyzing codebase index...
          </div>
        )}
      </div>

      {/* Input Box */}
      <div className="relative">
        <input
          type="text"
          value={inputPrompt}
          onChange={(e) => setInputPrompt(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && handleSend()}
          placeholder="Ask anything about this codebase..."
          className="w-full bg-white dark:bg-[#1C2128] border border-gray-300 dark:border-gray-800 rounded-xl pl-4 pr-12 py-3.5 text-xs text-gray-900 dark:text-white placeholder-gray-400 focus:outline-none focus:border-[#E67E22] transition-all shadow-sm"
        />
        <button
          onClick={() => handleSend()}
          className="absolute right-3 top-3 p-1.5 rounded-lg bg-[#E67E22] hover:bg-[#D35400] text-white transition-colors"
        >
          <Send size={14} />
        </button>
      </div>
    </div>
  );
};
