# GridSeak Core System Mental Model

```mermaid
%%{init: {
  "theme": "base",
  "flowchart": {
    "curve": "stepBefore",
    "nodeSpacing": 48,
    "rankSpacing": 70
  },
  "themeVariables": {
    "background": "#000000",
    "mainBkg": "#000000",
    "primaryColor": "#000000",
    "primaryTextColor": "#39ff14",
    "primaryBorderColor": "#39ff14",
    "lineColor": "#39ff14",
    "clusterBkg": "#000000",
    "clusterBorder": "#39ff14",
    "edgeLabelBackground": "#000000",
    "fontFamily": "Menlo, Monaco, Consolas, monospace"
  }
}}%%
flowchart TB
  Repo["Your repo<br/>source files + git history"]

  subgraph ScanPath["Scan path"]
    Status["status / freshness check<br/>Is there a usable recent scan?"]
    Parser["Parser<br/>tree-sitter + rust-analyzer/LSP<br/>extracts symbols, imports, calls"]
    ParseDB["Parse database<br/>SQLite graph artifact<br/>nodes + edges"]
    Analyzer["Analyzer<br/>cycles, coupling, cohesion,<br/>depth, blast radius, dead code, complexity"]
    Report["Health report<br/>scores, findings, recommendations"]
  end

  subgraph StorePath["Persistent local memory"]
    Store["Local GridSeak store<br/>scan history + report paths<br/>graph artifact paths + summaries"]
  end

  subgraph QueryPath["Cheap path: reuse stored facts"]
    MCP["MCP tools<br/>context, status, recommendations,<br/>callers, callees, cycles, blast radius"]
    CLI["CLI commands<br/>reports, metrics, graph queries"]
    GraphQueries["Graph queries<br/>many reads from the same SQLite graph<br/>no rescan needed while fresh"]
  end

  subgraph ReleasePath["Release boundary"]
    Private["Private source repo<br/>gridseak-graphengine"]
    AllowList["Public allow-list<br/>scripts/release/include-public.txt"]
    Public["Public mirror<br/>gridseak"]
  end

  Repo --> Status
  Status -->|fresh scan exists| Store
  Status -->|missing or stale| Parser
  Parser --> ParseDB
  ParseDB --> Analyzer
  Analyzer --> Report
  ParseDB --> Store
  Report --> Store

  Store --> MCP
  Store --> CLI
  MCP --> GraphQueries
  CLI --> GraphQueries
  GraphQueries --> ParseDB

  Private --> AllowList
  AllowList --> Public

  MCP --> Decisions["Agent answers from graph facts<br/>instead of re-grepping or guessing"]
  CLI --> Decisions

  classDef node fill:#000000,stroke:#39ff14,color:#39ff14,stroke-width:2px;
  classDef core fill:#001400,stroke:#39ff14,color:#39ff14,stroke-width:3px;
  classDef private fill:#000000,stroke:#00cc66,color:#00ff88,stroke-width:2px;

  class Repo,Status,Store,MCP,CLI,GraphQueries,Decisions node;
  class Parser,ParseDB,Analyzer,Report core;
  class Private,AllowList,Public private;
```

## One-Sentence Model

GridSeak performs an expensive scan only when facts are missing or stale, stores the resulting graph/report locally, and then serves many cheap MCP/CLI queries from that stored graph.

## Component Roles

| Component | Mental model |
| --- | --- |
| Status / freshness check | Decides whether the existing scan can be trusted or a new scan is needed. |
| Parser | Converts code into structural facts. |
| Parse database | The durable graph map of what depends on what. |
| Analyzer | Scores the map and finds risks. |
| Local store | Keeps scans, reports, and history on your machine. |
| MCP / CLI | Query surfaces over the same local facts without rescanning each time. |
| Public mirror | A curated subset copied from the private source repo. |

## Key Separation

| Path | Cost | When it runs |
| --- | --- | --- |
| Scan path | Expensive | First time, after meaningful source changes, or when cache is stale. |
| Query path | Cheap | Repeatedly, whenever an agent or human asks questions about the latest graph. |
