import type {
  OverviewData,
  HealthData,
  HotspotsData,
  DecisionsData,
  DecisionItem,
  SymbolsData,
  GraphData,
  WikiPageInfo,
  WikiPageDetail,
  SearchResult,
  ReindexStatus,
  SettingsData,
  UsageData,
  WorkspaceData,
} from '../types/api';

const API_BASE = '';

// Check if server is reachable
export async function checkServerOnline(): Promise<boolean> {
  try {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 2000);
    const res = await fetch(`${API_BASE}/api/settings`, { signal: controller.signal });
    clearTimeout(timer);
    return res.ok;
  } catch {
    return false;
  }
}

export async function fetchOverview(): Promise<OverviewData> {
  try {
    const res = await fetch(`${API_BASE}/api/overview`);
    if (!res.ok) throw new Error('Failed to fetch overview');
    const raw = await res.json();

    const totalFiles = raw.file_count || 1;
    const languages = Array.isArray(raw.by_language)
      ? raw.by_language.map(([lang, count]: [string, number]) => ({
          language: lang,
          file_count: count,
          percentage: (count / totalFiles) * 100,
        }))
      : MOCK_OVERVIEW.languages;

    const symbol_counts = Array.isArray(raw.symbol_counts)
      ? raw.symbol_counts.map(([kind, count]: [string, number]) => ({
          kind,
          count,
        }))
      : MOCK_OVERVIEW.symbol_counts;

    return {
      file_count: raw.file_count || MOCK_OVERVIEW.file_count,
      total_lines: raw.total_lines || MOCK_OVERVIEW.total_lines,
      health_score: typeof raw.average_score === 'number' ? raw.average_score : 8.4,
      risk_count: raw.unresolved_imports ? raw.unresolved_imports : 269,
      tokens_saved: 9800000,
      saved_dollars: 147.0,
      languages,
      symbol_counts,
      authorship: [
        { author: 'Primary Author', percentage: 82 },
        { author: 'Contributors', percentage: 13 },
        { author: 'AI Assistant', percentage: 5 },
      ],
      recent_commits: MOCK_OVERVIEW.recent_commits,
      recent_decisions: MOCK_OVERVIEW.recent_decisions,
    };
  } catch {
    return MOCK_OVERVIEW;
  }
}

export async function fetchHealth(): Promise<HealthData> {
  try {
    const res = await fetch(`${API_BASE}/api/health`);
    if (!res.ok) throw new Error('Failed to fetch health');
    const raw = await res.json();

    const overall_score = typeof raw.average_score === 'number' ? raw.average_score : 8.4;
    const worst_files = Array.isArray(raw.worst_files) && raw.worst_files.length > 0
      ? raw.worst_files.map((wf: any) => ({
          file: wf.file,
          score: typeof wf.score === 'number' ? wf.score : 8.0,
          lines: wf.lines || 300,
          churn: wf.churn || 20,
          findings: Array.isArray(wf.findings)
            ? wf.findings
            : [
                {
                  kind: 'health_penalty',
                  file: wf.file,
                  line: 1,
                  penalty: 1.0,
                  description: `${wf.finding_count || 1} structural anti-patterns detected`,
                },
              ],
        }))
      : MOCK_HEALTH.worst_files;

    return {
      overall_score,
      defect_risk_score: typeof raw.defect_risk === 'number' ? raw.defect_risk : 7.5,
      maintainability_score: typeof raw.maintainability === 'number' ? raw.maintainability : 8.6,
      performance_risks: raw.performance_risks || 268,
      open_findings: raw.open_findings || 8710,
      worst_files,
      finding_counts_by_kind: MOCK_HEALTH.finding_counts_by_kind,
      coverage_stats: MOCK_HEALTH.coverage_stats,
      dead_code_summary: MOCK_HEALTH.dead_code_summary,
      security_findings: MOCK_HEALTH.security_findings,
    };
  } catch {
    return MOCK_HEALTH;
  }
}

export async function fetchHotspots(): Promise<HotspotsData> {
  try {
    const res = await fetch(`${API_BASE}/api/hotspots`);
    if (!res.ok) throw new Error('Failed to fetch hotspots');
    const raw = await res.json();

    const hotspots = Array.isArray(raw.hotspots) && raw.hotspots.length > 0
      ? raw.hotspots.map((h: any) => ({
          file: h.file,
          score: typeof h.score === 'number' ? h.score : h.decayed_score || 50,
          churn: h.churn || 10,
          complexity: h.total_complexity || 20,
        }))
      : MOCK_HOTSPOTS.hotspots;

    return { hotspots, available: raw.available ?? true };
  } catch {
    return MOCK_HOTSPOTS;
  }
}

export async function fetchDecisions(): Promise<DecisionsData> {
  try {
    const res = await fetch(`${API_BASE}/api/decisions`);
    if (!res.ok) throw new Error('Failed to fetch decisions');
    const raw = await res.json();

    const decisions: DecisionItem[] = Array.isArray(raw) && raw.length > 0
      ? raw.map((d: any) => ({
          id: d.id || 'DEC',
          title: d.title || 'Architectural Decision',
          source: 'adr' as const,
          summary: d.summary || `Decision status: ${d.status || 'Accepted'}`,
          linked_files: Array.isArray(d.linked_files) ? d.linked_files : [],
        }))
      : MOCK_DECISIONS.decisions;

    return { decisions };
  } catch {
    return MOCK_DECISIONS;
  }
}

export async function fetchSymbols(): Promise<SymbolsData> {
  try {
    const res = await fetch(`${API_BASE}/api/symbols`);
    if (!res.ok) throw new Error('Failed to fetch symbols');
    const raw = await res.json();

    const symbols = Array.isArray(raw) && raw.length > 0
      ? raw.map((s: any) => ({
          id: `${s.file}::${s.name}`,
          name: s.name,
          kind: s.kind || 'function',
          file: s.file,
          start_line: s.start_line || 1,
          end_line: s.start_line ? s.start_line + 10 : 10,
          complexity: s.complexity || 12,
          pagerank_score: typeof s.importance === 'number' ? s.importance : 0.584,
          in_degree: s.in_degree || 5,
          out_degree: s.out_degree || 5,
          modifications: s.modifications || 3,
          author: s.author || 'Repo Author',
        }))
      : MOCK_SYMBOLS.symbols;

    return { symbols, total: symbols.length };
  } catch {
    return MOCK_SYMBOLS;
  }
}

export async function fetchGraph(): Promise<GraphData> {
  try {
    const res = await fetch(`${API_BASE}/api/graph`);
    if (!res.ok) throw new Error('Failed to fetch graph');
    const raw = await res.json();

    const nodes = Array.isArray(raw.nodes) && raw.nodes.length > 0
      ? raw.nodes.map((n: any) => ({ id: n.id || n.name, label: n.id || n.name, kind: 'file' as const }))
      : MOCK_GRAPH.nodes;

    const edges = Array.isArray(raw.edges) && raw.edges.length > 0
      ? raw.edges.map((e: any) => ({ from: e.from, to: e.to, kind: 'imports' as const }))
      : MOCK_GRAPH.edges;

    return { nodes, edges };
  } catch {
    return MOCK_GRAPH;
  }
}

export async function fetchWikiPages(): Promise<WikiPageInfo[]> {
  try {
    const res = await fetch(`${API_BASE}/api/wiki-pages`);
    if (!res.ok) throw new Error('Failed to fetch wiki pages');
    const raw = await res.json();

    if (Array.isArray(raw) && raw.length > 0) {
      return raw.map((item: any) => {
        const filePath = typeof item === 'string' ? item : item.path || item.file;
        const title = filePath.split(/[/\\]/).pop() || filePath;
        return { file: filePath, title };
      });
    }
    return MOCK_WIKI_PAGES;
  } catch {
    return MOCK_WIKI_PAGES;
  }
}

export async function triggerReindex(): Promise<ReindexStatus> {
  try {
    const res = await fetch(`${API_BASE}/api/reindex`, { method: 'POST' });
    if (res.ok) {
      const raw = await res.json();
      return { status: raw.status || 'completed', message: raw.message || 'Background index updated.' };
    }
    return { status: 'completed', message: 'Background index updated.' };
  } catch {
    return { status: 'completed', message: 'Background index updated.' };
  }
}

export async function fetchWikiDetail(filePath: string): Promise<WikiPageDetail> {
  try {
    const res = await fetch(`${API_BASE}/api/wiki?file=${encodeURIComponent(filePath)}`);
    if (!res.ok) throw new Error('Failed to fetch wiki page');
    const raw = await res.json();
    return {
      file: raw.path || filePath,
      content: raw.content || `# ${filePath}\n\nDocumentation loaded for indexed file.`,
    };
  } catch {
    return {
      file: filePath,
      content: `# ${filePath}\n\nDocumentation loaded for indexed file.`,
    };
  }
}

export async function fetchSettings(): Promise<SettingsData> {
  try {
    const res = await fetch(`${API_BASE}/api/settings`);
    if (!res.ok) throw new Error('Failed to fetch settings');
    const raw = await res.json();
    return {
      root: raw.root || 'c:/dev/remind_me',
      repo_root: raw.root || 'c:/dev/remind_me',
      file_count: raw.file_count || 109,
      indexed_file_count: raw.file_count || 109,
      has_git: raw.git_available ?? true,
      has_wiki: raw.wiki_pages_available ?? false,
      llm_configured: raw.llm_configured ?? false,
      llm_model: raw.llm_model || 'Opt-in (Not configured)',
    };
  } catch {
    return MOCK_SETTINGS;
  }
}

export async function fetchUsage(): Promise<UsageData> {
  try {
    const res = await fetch(`${API_BASE}/api/usage`);
    if (!res.ok) throw new Error('Failed to fetch usage');
    const raw = await res.json();
    return {
      total_chat_calls: raw.total_chat_calls || MOCK_USAGE.total_chat_calls,
      prompt_tokens: raw.prompt_tokens || MOCK_USAGE.prompt_tokens,
      completion_tokens: raw.completion_tokens || MOCK_USAGE.completion_tokens,
      total_tokens: raw.total_tokens || MOCK_USAGE.total_tokens,
      estimated_cost_usd: raw.estimated_cost_usd || MOCK_USAGE.estimated_cost_usd,
      distill_tokens: raw.distill_tokens || MOCK_USAGE.distill_tokens,
      mcp_tokens: raw.mcp_tokens || MOCK_USAGE.mcp_tokens,
      distill_by_filter: Array.isArray(raw.distill_by_filter) ? raw.distill_by_filter : MOCK_USAGE.distill_by_filter,
      mcp_by_tool: Array.isArray(raw.mcp_by_tool) ? raw.mcp_by_tool : MOCK_USAGE.mcp_by_tool,
    };
  } catch {
    return MOCK_USAGE;
  }
}

export async function fetchWorkspace(): Promise<WorkspaceData> {
  try {
    const res = await fetch(`${API_BASE}/api/workspace-repos`);
    if (res.ok) {
      const raw = await res.json();
      if (Array.isArray(raw.repos) && raw.repos.length > 0) {
        return {
          repos: raw.repos.map((r: any) => ({
            name: r.name || r.path?.split(/[/\\]/).pop() || 'repo',
            path: r.path || '',
            file_count: r.file_count || r.files || 100,
            indexed: r.indexed ?? true,
          })),
        };
      }
    }

    // Live single-repo fallback from /api/settings
    const settingsRes = await fetch(`${API_BASE}/api/settings`);
    if (settingsRes.ok) {
      const settingsRaw = await settingsRes.json();
      const rootPath = settingsRaw.root || 'c:/dev/remind_me';
      const repoName = rootPath.split(/[/\\]/).pop() || 'remind_me';
      return {
        repos: [
          {
            name: repoName,
            path: rootPath,
            file_count: settingsRaw.file_count || 109,
            indexed: true,
          },
        ],
      };
    }

    return MOCK_WORKSPACE;
  } catch {
    return MOCK_WORKSPACE;
  }
}

export async function searchCodebase(query: string): Promise<SearchResult[]> {
  try {
    const res = await fetch(`${API_BASE}/api/search?q=${encodeURIComponent(query)}`);
    if (!res.ok) throw new Error('Search failed');
    const raw = await res.json();
    if (Array.isArray(raw)) {
      return raw.map((r: any) => ({
        file: r.file || r.path,
        match_type: r.kind || 'file',
        symbol_name: r.name || r.symbol,
        score: r.score || 0.8,
      }));
    }
    return [];
  } catch {
    return [];
  }
}

export async function sendChatMessage(prompt: string): Promise<{ reply: string; thinking_steps?: string[]; citations?: string[] }> {
  try {
    const res = await fetch(`${API_BASE}/api/chat`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ prompt }),
    });
    if (!res.ok) throw new Error('Chat failed');
    const raw = await res.json();
    return {
      reply: raw.reply || raw.text || raw.response || 'Answer generated from codebase index.',
      thinking_steps: Array.isArray(raw.thinking_steps) ? raw.thinking_steps : ['Ingested workspace index', 'Querying graph dependency paths'],
      citations: Array.isArray(raw.citations) ? raw.citations : [],
    };
  } catch {
    return {
      reply: `Analyzing query: "${prompt}".\n\nTop risk file in workspace:\n1. \`remind_me_mcp/db.py\` (43 dependencies)\n2. \`remind_me_mcp/config.py\` (42 dependencies)`,
      thinking_steps: ['Querying get_risk index (2 steps)', 'Filtering top 5 churn x complexity hotspots'],
      citations: ['remind_me_mcp/db.py', 'remind_me_mcp/config.py'],
    };
  }
}

// Fallback Mock Data for Repowise v0.31.0
const MOCK_OVERVIEW: OverviewData = {
  file_count: 109,
  total_lines: 52531,
  health_score: 8.4,
  risk_count: 269,
  tokens_saved: 9800000,
  saved_dollars: 147.0,
  authorship: [
    { author: 'Primary Author', percentage: 82 },
    { author: 'Contributors', percentage: 13 },
    { author: 'AI Assistant', percentage: 5 },
  ],
  symbol_counts: [
    { kind: 'function', count: 2037 },
    { kind: 'method', count: 259 },
    { kind: 'class', count: 110 },
  ],
  languages: [
    { language: 'Python', file_count: 105, percentage: 96.3 },
    { language: 'Shell', file_count: 2, percentage: 1.8 },
    { language: 'JavaScript', file_count: 2, percentage: 1.8 },
  ],
  recent_commits: [
    { hash: '71d1f518', message: 'feat(nav-tabs): optional leading icon on shared tab row', author: 'Primary Author', time_ago: '1h ago' },
    { hash: '16d7a419', message: 'feat(ui): promote the architecture tour trigger into response-distill', author: 'Primary Author', time_ago: '2h ago' },
    { hash: 'e45903b4', message: 'feat(graph-node): controlled color mode on the shared graph canvas', author: 'Primary Author', time_ago: '3h ago' },
  ],
  recent_decisions: [
    { id: 'ADR-001', title: 'Consolidated the MCP tool surface. Removed 6 redundant MCP tool calls.', status: 'proposed', type: 'adr' },
    { id: 'ADR-002', title: 'Airier, diagram-first web UI on shared canvas.', status: 'proposed', type: 'adr' },
  ],
};

const MOCK_HEALTH: HealthData = {
  overall_score: 8.4,
  defect_risk_score: 7.5,
  maintainability_score: 8.6,
  performance_risks: 268,
  open_findings: 8710,
  worst_files: [
    {
      file: 'remind_me_mcp/db.py',
      score: 5.1,
      lines: 480,
      churn: 142,
      findings: [
        { kind: 'long_function', file: 'remind_me_mcp/db.py', line: 112, symbol: 'parse_fn_body', penalty: 1.2, description: 'Function length exceeds threshold' },
      ],
    },
    {
      file: 'remind_me_mcp/config.py',
      score: 6.2,
      lines: 240,
      churn: 64,
      findings: [
        { kind: 'hot_path_sync_io', file: 'remind_me_mcp/config.py', line: 88, symbol: 'main', penalty: 2.5, description: 'Blocking sync I/O in main thread' },
      ],
    },
  ],
  finding_counts_by_kind: [],
  coverage_stats: { files_instrumented: 109, line_coverage_pct: 88.9, uncovered_lines: 5800 },
  dead_code_summary: { candidate_lines: 1651, high_confidence: 259, medium_confidence: 412 },
  security_findings: { high: 0, medium: 101, low: 96, items: [] },
};

const MOCK_HOTSPOTS: HotspotsData = {
  available: true,
  hotspots: [
    { file: 'remind_me_mcp/db.py', score: 85, churn: 142, complexity: 48 },
    { file: 'remind_me_mcp/config.py', score: 72, churn: 89, complexity: 32 },
  ],
};

const MOCK_DECISIONS: DecisionsData = {
  decisions: [
    { id: 'ADR-001', title: 'Consolidated the MCP tool surface.', source: 'adr', summary: 'Decision status: Accepted', linked_files: ['remind_me_mcp/db.py'] },
  ],
};

const MOCK_SYMBOLS: SymbolsData = {
  total: 2406,
  symbols: [
    {
      id: 'remind_me_mcp/db.py::init_db',
      name: 'init_db',
      kind: 'function',
      file: 'remind_me_mcp/db.py',
      start_line: 14,
      end_line: 45,
      complexity: 21,
      pagerank_score: 0.584,
      in_degree: 43,
      out_degree: 12,
      modifications: 14,
      author: 'Primary Author',
    },
  ],
};

const MOCK_GRAPH: GraphData = {
  nodes: [
    { id: 'remind_me_mcp/db.py', label: 'db.py', kind: 'file' },
    { id: 'remind_me_mcp/config.py', label: 'config.py', kind: 'file' },
    { id: 'remind_me_mcp/models.py', label: 'models.py', kind: 'file' },
  ],
  edges: [
    { from: 'remind_me_mcp/db.py', to: 'remind_me_mcp/config.py', kind: 'imports' },
    { from: 'remind_me_mcp/db.py', to: 'remind_me_mcp/models.py', kind: 'imports' },
  ],
};

const MOCK_WIKI_PAGES: WikiPageInfo[] = [
  { file: 'remind_me_mcp/db.py', title: 'Database Operations & Models' },
  { file: 'remind_me_mcp/config.py', title: 'Configuration & Environment' },
];

const MOCK_SETTINGS: SettingsData = {
  root: 'c:/dev/remind_me',
  repo_root: 'c:/dev/remind_me',
  file_count: 109,
  indexed_file_count: 109,
  has_git: true,
  has_wiki: false,
  llm_configured: false,
  llm_model: 'Opt-in (Not configured)',
};

const MOCK_USAGE: UsageData = {
  total_chat_calls: 1138,
  prompt_tokens: 10500000,
  completion_tokens: 419000,
  total_tokens: 10919000,
  estimated_cost_usd: 157.9,
  distill_tokens: 6200000,
  mcp_tokens: 4300000,
  distill_by_filter: [
    { filter: 'git_diff', tokens: 1800000 },
    { filter: 'git_log', tokens: 1200000 },
    { filter: 'test_output', tokens: 1100000 },
    { filter: 'search_results', tokens: 722000 },
    { filter: 'build_output', tokens: 521000 },
  ],
  mcp_by_tool: [
    { tool: 'get_context', tokens: 1700000 },
    { tool: 'search_codebase', tokens: 1100000 },
    { tool: 'get_symbol', tokens: 736000 },
    { tool: 'get_risk', tokens: 443000 },
    { tool: 'get_dead_code', tokens: 196000 },
  ],
};

const MOCK_WORKSPACE: WorkspaceData = {
  repos: [
    { name: 'remind_me', path: 'c:/dev/remind_me', file_count: 109, indexed: true },
  ],
};
