export interface SymbolCount {
  kind: string;
  count: number;
}

export interface LanguageDistribution {
  language: string;
  file_count: number;
  percentage: number;
}

export interface OverviewData {
  file_count: number;
  symbol_counts: SymbolCount[];
  total_lines: number;
  languages: LanguageDistribution[];
  health_score: number;
  risk_count: number;
  tokens_saved: number;
  saved_dollars: number;
  authorship: {
    author: string;
    percentage: number;
  }[];
  recent_commits: {
    hash: string;
    message: string;
    author: string;
    time_ago: string;
  }[];
  recent_decisions: {
    id: string;
    title: string;
    status: string;
    type: string;
  }[];
}

export interface HealthFinding {
  kind: string;
  file: string;
  line: number;
  symbol?: string;
  penalty: number;
  description: string;
}

export interface FileHealthScore {
  file: string;
  score: number;
  findings: HealthFinding[];
  lines?: number;
  churn?: number;
}

export interface HealthData {
  overall_score: number;
  defect_risk_score: number;
  maintainability_score: number;
  performance_risks: number;
  open_findings: number;
  worst_files: FileHealthScore[];
  finding_counts_by_kind: { kind: string; count: number }[];
  coverage_stats: {
    files_instrumented: number;
    line_coverage_pct: number;
    uncovered_lines: number;
  };
  dead_code_summary: {
    candidate_lines: number;
    high_confidence: number;
    medium_confidence: number;
  };
  security_findings: {
    high: number;
    medium: number;
    low: number;
    items: { file: string; line: number; kind: string; snippet: string; severity: 'high' | 'medium' | 'low' }[];
  };
}

export interface HotspotItem {
  file: string;
  score: number;
  churn: number;
  complexity: number;
}

export interface HotspotsData {
  hotspots: HotspotItem[];
  available: boolean;
}

export interface DecisionItem {
  id: string;
  title: string;
  source: 'adr' | 'commit' | 'comment' | 'pr';
  summary: string;
  linked_files: string[];
  status?: string;
}

export interface DecisionsData {
  decisions: DecisionItem[];
}

export interface SymbolItem {
  id: string;
  name: string;
  kind: string;
  file: string;
  start_line: number;
  end_line?: number;
  complexity?: number;
  pagerank_score?: number;
  in_degree?: number;
  out_degree?: number;
  modifications?: number;
  author?: string;
}

export interface SymbolsData {
  symbols: SymbolItem[];
  total: number;
}

export interface GraphNode {
  id: string;
  label: string;
  kind: 'file' | 'symbol';
  community?: string;
}

export interface GraphEdge {
  from: string;
  to: string;
  kind: 'imports' | 'calls' | 'contains';
}

export interface GraphData {
  nodes: GraphNode[];
  edges: GraphEdge[];
}

export interface DeadCodeCandidate {
  symbol_id: string;
  symbol_name: string;
  file: string;
  line: number;
  confidence: 'high' | 'medium' | 'low';
  reasons: string[];
  lines_saved?: number;
}

export interface DeadCodeData {
  candidates: DeadCodeCandidate[];
}

export interface WikiPageInfo {
  file: string;
  title: string;
}

export interface WikiPageDetail {
  file: string;
  content: string;
}

export interface SearchResult {
  file: string;
  symbol_name?: string;
  kind?: string;
  line?: number;
  score: number;
  match_type: 'file' | 'symbol';
}

export interface ReindexStatus {
  status: 'idle' | 'running' | 'completed' | 'failed';
  started_at?: string;
  completed_at?: string;
  message?: string;
}

export interface SettingsData {
  root?: string;
  repo_root: string;
  file_count: number;
  indexed_file_count: number;
  has_git: boolean;
  has_wiki: boolean;
  llm_configured: boolean;
  llm_model?: string;
}

export interface UsageData {
  total_chat_calls: number;
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  estimated_cost_usd: number;
  distill_tokens: number;
  mcp_tokens: number;
  distill_by_filter: { filter: string; tokens: number }[];
  mcp_by_tool: { tool: string; tokens: number }[];
}

export interface ChatMessage {
  id: string;
  sender: 'user' | 'assistant';
  text: string;
  timestamp: string;
  thinking_steps?: string[];
  citations?: string[];
}

export interface WorkspaceRepoInfo {
  name: string;
  path?: string;
  indexed: boolean;
  file_count?: number;
}

export interface WorkspaceData {
  repos: WorkspaceRepoInfo[];
}

export interface CommitHistoryItem {
  hash: string;
  message: string;
  author: string;
  lines_changed: number;
  date: string;
  risk_level: 'high' | 'medium' | 'low';
  category: 'feature' | 'fix' | 'refactor' | 'docs' | 'test' | 'chore';
}

export interface CommitsData {
  commits: CommitHistoryItem[];
  total_commits: number;
  high_priority: number;
  fix_commits: number;
  avg_entropy: number;
}
