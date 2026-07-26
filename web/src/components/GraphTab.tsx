import React, { useState, useMemo } from 'react';
import type { GraphData } from '../types/api';
import { Network, ArrowRight, Layers, LayoutGrid, Eye, ZoomIn, ZoomOut } from 'lucide-react';

interface GraphTabProps {
  graph: GraphData | null;
}

export const GraphTab: React.FC<GraphTabProps> = ({ graph }) => {
  const [selectedNode, setSelectedNode] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<'graph' | 'matrix'>('graph');
  const [zoom, setZoom] = useState<number>(1);

  if (!graph || graph.nodes.length === 0) {
    return (
      <div className="glass-panel p-12 rounded-xl border border-gray-800 text-center text-gray-400">
        No dependency graph nodes detected in codebase.
      </div>
    );
  }

  const activeNodeId = selectedNode || graph.nodes[0]?.id;

  // Filter top connected nodes for clean canvas display
  const displayNodes = useMemo(() => {
    // Keep max 30 nodes for clear visual rendering
    return graph.nodes.slice(0, 30);
  }, [graph.nodes]);

  const displayNodeIds = useMemo(() => new Set(displayNodes.map((n) => n.id)), [displayNodes]);

  // Compute 2D circular topology layout positions for SVG Graph Canvas
  const nodePositions = useMemo(() => {
    const map = new Map<string, { x: number; y: number; label: string }>();
    const count = displayNodes.length;
    const cx = 400;
    const cy = 260;
    const rx = 300;
    const ry = 190;

    displayNodes.forEach((node, i) => {
      const angle = (2 * Math.PI * i) / count - Math.PI / 2;
      const x = cx + rx * Math.cos(angle);
      const y = cy + ry * Math.sin(angle);
      const shortName = node.label.split(/[/\\]/).pop() || node.label;
      map.set(node.id, { x, y, label: shortName });
    });

    return map;
  }, [displayNodes]);

  // Edges matching display nodes
  const displayEdges = useMemo(() => {
    return graph.edges.filter((e) => displayNodeIds.has(e.from) && displayNodeIds.has(e.to));
  }, [graph.edges, displayNodeIds]);

  const outgoingEdges = graph.edges.filter((e) => e.from === activeNodeId);
  const incomingEdges = graph.edges.filter((e) => e.to === activeNodeId);

  return (
    <div className="space-y-6 animate-fade-in">
      {/* Header */}
      <div className="glass-panel p-6 rounded-xl border border-gray-800 flex flex-col md:flex-row md:items-center justify-between gap-4">
        <div>
          <h2 className="text-xl font-bold text-white flex items-center gap-2">
            <Network className="text-cyan-400" size={24} />
            File & Module Dependency Graph
          </h2>
          <p className="text-sm text-gray-400 mt-1">
            Visualizes import relationships, call dependencies, and module containment across the codebase.
          </p>
        </div>

        <div className="flex items-center gap-3">
          <div className="flex items-center bg-gray-900 rounded-lg p-1 border border-gray-800">
            <button
              onClick={() => setViewMode('graph')}
              className={`px-3 py-1.5 rounded-md text-xs font-semibold flex items-center gap-1.5 transition-colors ${
                viewMode === 'graph'
                  ? 'bg-cyan-500/20 text-cyan-300 border border-cyan-500/30'
                  : 'text-gray-400 hover:text-white'
              }`}
            >
              <Eye size={14} /> Network Graph
            </button>
            <button
              onClick={() => setViewMode('matrix')}
              className={`px-3 py-1.5 rounded-md text-xs font-semibold flex items-center gap-1.5 transition-colors ${
                viewMode === 'matrix'
                  ? 'bg-cyan-500/20 text-cyan-300 border border-cyan-500/30'
                  : 'text-gray-400 hover:text-white'
              }`}
            >
              <LayoutGrid size={14} /> Module Matrix
            </button>
          </div>
        </div>
      </div>

      {viewMode === 'graph' ? (
        <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
          {/* Interactive SVG Network Graph Canvas */}
          <div className="glass-panel p-5 rounded-xl border border-gray-800 lg:col-span-2 space-y-3 relative overflow-hidden">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <span className="w-2.5 h-2.5 rounded-full bg-cyan-400 animate-pulse" />
                <span className="text-xs font-mono font-semibold text-gray-300 uppercase tracking-wider">
                  Interactive Network Canvas ({displayNodes.length} nodes, {displayEdges.length} edges)
                </span>
              </div>
              <div className="flex items-center gap-2">
                <button
                  onClick={() => setZoom((z) => Math.max(z - 0.15, 0.7))}
                  className="p-1.5 rounded bg-gray-900 border border-gray-800 text-gray-400 hover:text-white"
                  title="Zoom Out"
                >
                  <ZoomOut size={14} />
                </button>
                <span className="text-xs font-mono text-gray-400">{Math.round(zoom * 100)}%</span>
                <button
                  onClick={() => setZoom((z) => Math.min(z + 0.15, 1.5))}
                  className="p-1.5 rounded bg-gray-900 border border-gray-800 text-gray-400 hover:text-white"
                  title="Zoom In"
                >
                  <ZoomIn size={14} />
                </button>
              </div>
            </div>

            {/* Canvas Diagram View */}
            <div className="w-full h-[520px] bg-gray-950/90 rounded-xl border border-gray-800/80 relative flex items-center justify-center overflow-hidden">
              <svg
                viewBox="0 0 800 520"
                className="w-full h-full cursor-grab"
                style={{ transform: `scale(${zoom})`, transformOrigin: 'center center', transition: 'transform 0.2s ease' }}
              >
                <defs>
                  <marker
                    id="arrowhead"
                    markerWidth="8"
                    markerHeight="6"
                    refX="14"
                    refY="3"
                    orient="auto"
                  >
                    <polygon points="0 0, 8 3, 0 6" fill="#3B82F6" opacity="0.6" />
                  </marker>
                  <marker
                    id="arrowhead-active"
                    markerWidth="10"
                    markerHeight="7"
                    refX="16"
                    refY="3.5"
                    orient="auto"
                  >
                    <polygon points="0 0, 10 3.5, 0 7" fill="#06B6D4" />
                  </marker>
                  <radialGradient id="nodeGlow" cx="50%" cy="50%" r="50%">
                    <stop offset="0%" stopColor="#06B6D4" stopOpacity="0.4" />
                    <stop offset="100%" stopColor="#06B6D4" stopOpacity="0" />
                  </radialGradient>
                </defs>

                {/* Connecting Dependency Edge Lines */}
                {displayEdges.map((edge, idx) => {
                  const fromPos = nodePositions.get(edge.from);
                  const toPos = nodePositions.get(edge.to);
                  if (!fromPos || !toPos) return null;

                  const isConnectedToActive = edge.from === activeNodeId || edge.to === activeNodeId;
                  const strokeColor = isConnectedToActive
                    ? edge.from === activeNodeId
                      ? '#3B82F6'
                      : '#10B981'
                    : '#1F2937';
                  const strokeWidth = isConnectedToActive ? 2.5 : 1;
                  const opacity = isConnectedToActive ? 0.9 : 0.25;

                  return (
                    <line
                      key={idx}
                      x1={fromPos.x}
                      y1={fromPos.y}
                      x2={toPos.x}
                      y2={toPos.y}
                      stroke={strokeColor}
                      strokeWidth={strokeWidth}
                      strokeOpacity={opacity}
                      markerEnd={isConnectedToActive ? 'url(#arrowhead-active)' : 'url(#arrowhead)'}
                    />
                  );
                })}

                {/* Node Circles */}
                {displayNodes.map((node) => {
                  const pos = nodePositions.get(node.id);
                  if (!pos) return null;

                  const isActive = node.id === activeNodeId;
                  const isConnected = displayEdges.some(
                    (e) => (e.from === activeNodeId && e.to === node.id) || (e.to === activeNodeId && e.from === node.id)
                  );

                  const circleColor = isActive
                    ? '#06B6D4'
                    : isConnected
                    ? '#3B82F6'
                    : '#1F2937';

                  return (
                    <g
                      key={node.id}
                      className="cursor-pointer transition-transform hover:scale-110"
                      onClick={() => setSelectedNode(node.id)}
                    >
                      {/* Halo Glow Ring on Active Node */}
                      {isActive && (
                        <circle cx={pos.x} cy={pos.y} r="28" fill="url(#nodeGlow)" />
                      )}

                      <circle
                        cx={pos.x}
                        cy={pos.y}
                        r={isActive ? 14 : 9}
                        fill={circleColor}
                        stroke={isActive ? '#FFFFFF' : '#374151'}
                        strokeWidth={isActive ? 2.5 : 1.5}
                      />

                      <text
                        x={pos.x}
                        y={pos.y + (isActive ? 26 : 20)}
                        textAnchor="middle"
                        fill={isActive ? '#F3F4F6' : '#9CA3AF'}
                        fontSize={isActive ? '11' : '9'}
                        fontFamily="monospace"
                        fontWeight={isActive ? 'bold' : 'normal'}
                      >
                        {pos.label}
                      </text>
                    </g>
                  );
                })}
              </svg>

              {/* Diagram Overlay Legend */}
              <div className="absolute bottom-3 left-3 bg-gray-900/90 border border-gray-800 rounded-lg p-2.5 text-[11px] font-mono flex items-center gap-4 text-gray-300">
                <span className="flex items-center gap-1.5">
                  <span className="w-2.5 h-2.5 rounded-full bg-cyan-400" /> Active Selected
                </span>
                <span className="flex items-center gap-1.5">
                  <span className="w-2.5 h-2.5 rounded-full bg-blue-500" /> Outgoing Import
                </span>
                <span className="flex items-center gap-1.5">
                  <span className="w-2.5 h-2.5 rounded-full bg-emerald-500" /> Incoming Dependent
                </span>
              </div>
            </div>
          </div>

          {/* Node Relationship Detail Panel */}
          <div className="glass-panel p-6 rounded-xl border border-gray-800 space-y-6">
            <div className="flex items-center justify-between pb-4 border-b border-gray-800">
              <div>
                <div className="text-xs text-cyan-400 font-semibold uppercase tracking-wider">Selected Module</div>
                <h3 className="text-sm font-mono font-bold text-white mt-1 break-all">{activeNodeId}</h3>
              </div>
            </div>

            <div className="space-y-6">
              {/* Outgoing Imports */}
              <div className="space-y-3">
                <h4 className="text-sm font-semibold text-gray-300 flex items-center justify-between">
                  <span className="flex items-center gap-2">
                    <ArrowRight size={16} className="text-blue-400" /> Imports
                  </span>
                  <span className="px-2 py-0.5 rounded text-xs font-mono font-bold bg-blue-500/10 text-blue-400 border border-blue-500/20">
                    {outgoingEdges.length}
                  </span>
                </h4>
                {outgoingEdges.length === 0 ? (
                  <div className="p-3 rounded-lg bg-gray-900/50 border border-gray-800 text-xs text-gray-500">
                    No outgoing imports.
                  </div>
                ) : (
                  <div className="space-y-2 max-h-[160px] overflow-y-auto pr-1">
                    {outgoingEdges.map((e, idx) => (
                      <div
                        key={idx}
                        onClick={() => setSelectedNode(e.to)}
                        className="p-2.5 rounded-lg bg-gray-900/80 border border-gray-800 flex items-center justify-between hover:border-blue-500/40 cursor-pointer"
                      >
                        <span className="font-mono text-xs text-blue-300 truncate max-w-[200px]">{e.to}</span>
                        <span className="text-[10px] uppercase font-bold text-gray-500">{e.kind}</span>
                      </div>
                    ))}
                  </div>
                )}
              </div>

              {/* Incoming Dependents */}
              <div className="space-y-3">
                <h4 className="text-sm font-semibold text-gray-300 flex items-center justify-between">
                  <span className="flex items-center gap-2">
                    <ArrowRight size={16} className="text-emerald-400 rotate-180" /> Dependents
                  </span>
                  <span className="px-2 py-0.5 rounded text-xs font-mono font-bold bg-emerald-500/10 text-emerald-400 border border-emerald-500/20">
                    {incomingEdges.length}
                  </span>
                </h4>
                {incomingEdges.length === 0 ? (
                  <div className="p-3 rounded-lg bg-gray-900/50 border border-gray-800 text-xs text-gray-500">
                    No incoming dependents.
                  </div>
                ) : (
                  <div className="space-y-2 max-h-[160px] overflow-y-auto pr-1">
                    {incomingEdges.map((e, idx) => (
                      <div
                        key={idx}
                        onClick={() => setSelectedNode(e.from)}
                        className="p-2.5 rounded-lg bg-gray-900/80 border border-gray-800 flex items-center justify-between hover:border-emerald-500/40 cursor-pointer"
                      >
                        <span className="font-mono text-xs text-emerald-300 truncate max-w-[200px]">{e.from}</span>
                        <span className="text-[10px] uppercase font-bold text-gray-500">{e.kind}</span>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </div>
          </div>
        </div>
      ) : (
        /* Matrix View */
        <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
          <div className="glass-panel p-5 rounded-xl border border-gray-800 space-y-3">
            <h3 className="text-sm font-semibold text-gray-300 uppercase tracking-wider flex items-center gap-2">
              <Layers size={16} className="text-blue-400" /> Code Modules ({graph.nodes.length})
            </h3>
            <div className="space-y-2 max-h-[500px] overflow-y-auto pr-1">
              {graph.nodes.map((n) => {
                const isSelected = activeNodeId === n.id;
                return (
                  <button
                    key={n.id}
                    onClick={() => setSelectedNode(n.id)}
                    className={`w-full text-left p-3 rounded-lg border transition-all flex items-center justify-between ${
                      isSelected
                        ? 'bg-cyan-500/20 border-cyan-500/50 text-white font-semibold'
                        : 'bg-gray-900/50 border-gray-800/80 text-gray-300 hover:bg-gray-800/60'
                    }`}
                  >
                    <span className="font-mono text-sm truncate">{n.label}</span>
                    <span className="px-2 py-0.5 rounded text-[10px] uppercase font-bold bg-blue-500/10 text-blue-400 border border-blue-500/20">
                      {n.kind}
                    </span>
                  </button>
                );
              })}
            </div>
          </div>

          <div className="glass-panel p-6 rounded-xl border border-gray-800 lg:col-span-2 space-y-6">
            <div className="flex items-center justify-between pb-4 border-b border-gray-800">
              <div>
                <div className="text-xs text-cyan-400 font-semibold uppercase tracking-wider">Selected Module</div>
                <h3 className="text-lg font-mono font-bold text-white mt-1">{activeNodeId}</h3>
              </div>
            </div>

            <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
              <div className="space-y-3">
                <h4 className="text-sm font-semibold text-gray-300 flex items-center gap-2">
                  <ArrowRight size={16} className="text-blue-400" /> Imports ({outgoingEdges.length})
                </h4>
                {outgoingEdges.length === 0 ? (
                  <div className="p-4 rounded-lg bg-gray-900/50 border border-gray-800 text-xs text-gray-500">
                    No outgoing imports.
                  </div>
                ) : (
                  outgoingEdges.map((e, idx) => (
                    <div key={idx} className="p-3 rounded-lg bg-gray-900/80 border border-gray-800 flex items-center justify-between">
                      <span className="font-mono text-sm text-blue-300">{e.to}</span>
                      <span className="text-[10px] uppercase font-bold text-gray-400">{e.kind}</span>
                    </div>
                  ))
                )}
              </div>

              <div className="space-y-3">
                <h4 className="text-sm font-semibold text-gray-300 flex items-center gap-2">
                  <ArrowRight size={16} className="text-emerald-400 rotate-180" /> Dependents ({incomingEdges.length})
                </h4>
                {incomingEdges.length === 0 ? (
                  <div className="p-4 rounded-lg bg-gray-900/50 border border-gray-800 text-xs text-gray-500">
                    No incoming dependents.
                  </div>
                ) : (
                  incomingEdges.map((e, idx) => (
                    <div key={idx} className="p-3 rounded-lg bg-gray-900/80 border border-gray-800 flex items-center justify-between">
                      <span className="font-mono text-sm text-emerald-300">{e.from}</span>
                      <span className="text-[10px] uppercase font-bold text-gray-400">{e.kind}</span>
                    </div>
                  ))
                )}
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};
