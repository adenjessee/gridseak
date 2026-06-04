//! Concrete graph queries used by the `gridseak graph *` commands.
//!
//! Each function takes an open read-only `Connection` and returns a
//! Rust-native result type. The renderer (see
//! `crate::render::graph`) is responsible for formatting; this module
//! is responsible for the SQL.

use std::collections::HashSet;

use rusqlite::Connection;

use std::collections::HashMap;

use super::{
    best_tier, node_is_function, walk_calls, CoupledModuleRow, CycleRow, Direction, FanRow,
    FileBlastRadiusResult, FileBlastRadiusRow, GraphQueryError, NodeRef, SliceDirection, SliceNode,
};

// ---------------------------------------------------------------------------
// Functions ranked by Call-edge fan-in / fan-out
// ---------------------------------------------------------------------------

pub fn top_fan_in(conn: &Connection, limit: usize) -> Result<Vec<FanRow>, GraphQueryError> {
    let mut stmt = conn.prepare(
        "SELECT n.id, n.fqn, n.kind, COUNT(*) AS fan_in
         FROM edges e
         JOIN nodes n ON n.id = e.to_id
         WHERE json_extract(e.kind, '$.kind') = 'Call' AND n.kind = 'Function'
         GROUP BY e.to_id
         ORDER BY fan_in DESC, n.fqn ASC
         LIMIT ?1",
    )?;
    let rows = stmt
        .query_map([limit as i64], |row| {
            Ok(FanRow {
                node: NodeRef {
                    id: row.get(0)?,
                    fqn: row.get(1)?,
                    kind: row.get(2)?,
                },
                count: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn top_fan_out(conn: &Connection, limit: usize) -> Result<Vec<FanRow>, GraphQueryError> {
    let mut stmt = conn.prepare(
        "SELECT n.id, n.fqn, n.kind, COUNT(*) AS fan_out
         FROM edges e
         JOIN nodes n ON n.id = e.from_id
         WHERE json_extract(e.kind, '$.kind') = 'Call' AND n.kind = 'Function'
         GROUP BY e.from_id
         ORDER BY fan_out DESC, n.fqn ASC
         LIMIT ?1",
    )?;
    let rows = stmt
        .query_map([limit as i64], |row| {
            Ok(FanRow {
                node: NodeRef {
                    id: row.get(0)?,
                    fqn: row.get(1)?,
                    kind: row.get(2)?,
                },
                count: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Hotspots = high fan-in functions. The threshold matches the
/// analysis crate's heuristic (≥ 10 callers) but we expose it as a
/// CLI flag for power users. Returns rows in descending fan-in.
pub fn hotspots(
    conn: &Connection,
    min_fan_in: i64,
    limit: usize,
) -> Result<Vec<FanRow>, GraphQueryError> {
    let mut stmt = conn.prepare(
        "SELECT n.id, n.fqn, n.kind, COUNT(*) AS fan_in
         FROM edges e
         JOIN nodes n ON n.id = e.to_id
         WHERE json_extract(e.kind, '$.kind') = 'Call' AND n.kind = 'Function'
         GROUP BY e.to_id
         HAVING fan_in >= ?1
         ORDER BY fan_in DESC, n.fqn ASC
         LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![min_fan_in, limit as i64], |row| {
            Ok(FanRow {
                node: NodeRef {
                    id: row.get(0)?,
                    fqn: row.get(1)?,
                    kind: row.get(2)?,
                },
                count: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Dead code: Function nodes with zero incoming Call edges
// ---------------------------------------------------------------------------

/// Dead-code candidates from the graph perspective: functions with no
/// inbound `Call` edges and not on a list of well-known entrypoint
/// names. This is *graph-only* — the analysis crate's `DeadCode`
/// finding additionally consults LSP fidelity and ecosystem
/// classification, so this query is an approximation suitable for
/// "show me where to start digging" but not for "I am sure this is
/// dead". The renderer attaches that caveat.
pub fn dead_code(conn: &Connection, limit: usize) -> Result<Vec<FanRow>, GraphQueryError> {
    let mut stmt = conn.prepare(
        "SELECT n.id, n.fqn, n.kind, 0 AS fan_in
         FROM nodes n
         WHERE n.kind = 'Function'
           AND NOT EXISTS (
               SELECT 1 FROM edges e
               WHERE e.to_id = n.id AND json_extract(e.kind, '$.kind') = 'Call'
           )
         ORDER BY n.fqn ASC
         LIMIT ?1",
    )?;
    let rows = stmt
        .query_map([limit as i64], |row| {
            Ok(FanRow {
                node: NodeRef {
                    id: row.get(0)?,
                    fqn: row.get(1)?,
                    kind: row.get(2)?,
                },
                count: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Callers / Callees (immediate neighbours)
// ---------------------------------------------------------------------------

pub fn callers(conn: &Connection, target_id: &str) -> Result<Vec<NodeRef>, GraphQueryError> {
    let mut stmt = conn.prepare(
        "SELECT n.id, n.fqn, n.kind FROM edges e
         JOIN nodes n ON n.id = e.from_id
         WHERE e.to_id = ?1 AND json_extract(e.kind, '$.kind') = 'Call'
         ORDER BY n.fqn ASC",
    )?;
    let rows = stmt
        .query_map([target_id], |row| {
            Ok(NodeRef {
                id: row.get(0)?,
                fqn: row.get(1)?,
                kind: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn callees(conn: &Connection, source_id: &str) -> Result<Vec<NodeRef>, GraphQueryError> {
    let mut stmt = conn.prepare(
        "SELECT n.id, n.fqn, n.kind FROM edges e
         JOIN nodes n ON n.id = e.to_id
         WHERE e.from_id = ?1 AND json_extract(e.kind, '$.kind') = 'Call'
         ORDER BY n.fqn ASC",
    )?;
    let rows = stmt
        .query_map([source_id], |row| {
            Ok(NodeRef {
                id: row.get(0)?,
                fqn: row.get(1)?,
                kind: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Blast radius: BFS downstream from a seed
// ---------------------------------------------------------------------------

/// Returns the unique **upstream** Call-graph dependents within
/// `max_depth` hops — i.e. transitive callers of `seed_id`. This is
/// the semantic that answers "if I change X, what breaks?": the BFS
/// walks Call edges *in reverse* so every result is something that
/// would have to be re-validated when `seed_id` changes.
///
/// Historical note: a previous revision of this function walked the
/// forward direction (callees), which inverted the meaning of every
/// "blast radius" answer returned through the MCP surface. The
/// regression test `blast_radius_walks_callers_not_callees` pins the
/// direction so it cannot silently flip back. For the forward
/// direction ("what does X transitively reach"), call `slice` and
/// filter on `SliceDirection::Downstream`.
pub fn blast_radius(
    conn: &Connection,
    seed_id: &str,
    max_depth: usize,
    cap: usize,
) -> Result<Vec<SliceNode>, GraphQueryError> {
    // Refuse seeds that don't have Call edges in the graph. Without
    // this guard the BFS just returns an empty list, which callers
    // (especially LLM agents) misread as "nothing depends on this".
    let mut stmt = conn.prepare("SELECT fqn, kind FROM nodes WHERE id = ?1 LIMIT 1")?;
    let mut rows_iter = stmt.query([seed_id])?;
    if let Some(row) = rows_iter.next()? {
        let seed_fqn: String = row.get(0)?;
        let kind: String = row.get(1)?;
        let dummy = NodeRef {
            id: seed_id.to_string(),
            fqn: seed_fqn.clone(),
            kind: kind.clone(),
        };
        if !node_is_function(&dummy) {
            return Err(GraphQueryError::SeedNotAFunction { seed_fqn, kind });
        }
    }
    let rows = walk_calls(conn, seed_id, Direction::In, max_depth, cap)?;
    Ok(rows
        .into_iter()
        .map(|(depth, node, edge_evidence_tier)| SliceNode {
            depth,
            direction: SliceDirection::Upstream,
            node,
            edge_evidence_tier,
        })
        .collect())
}

// ---------------------------------------------------------------------------
// File-scoped blast radius: aggregated upstream BFS from every
// function in a single file. One call replaces the 50-symbol fanout
// agents previously had to do client-side (Q1 in the rc1 dogfood
// follow-up).
// ---------------------------------------------------------------------------

/// Returns the unioned upstream blast radius for every callable
/// symbol that physically lives in `file_path`. Useful answer to
/// "if I change file X, what breaks?" — the agent does not have to
/// enumerate the file's symbols and fan out per-symbol calls.
///
/// File-residence is computed from `nodes.location.file` (the
/// repo-relative path written by the parser at scan time). The file
/// path must match exactly; the renderer normalises it before
/// calling.
///
/// Each result row is deduped across seeds and carries the list of
/// file-resident symbols that pulled it in, so the caller can render
/// "affected because the file's init AND tick reach this node." A
/// row that only one seed reaches is "marginally affected"; one that
/// every seed reaches is "anyone-editing-this-file's problem."
///
/// Rows representing the seeds themselves are excluded from `rows`
/// so the caller sees only the *external* blast radius (intra-file
/// edges are noise here).
pub fn file_blast_radius(
    conn: &Connection,
    file_path: &str,
    max_depth: usize,
    cap_per_seed: usize,
) -> Result<FileBlastRadiusResult, GraphQueryError> {
    // Step 1: enumerate function-kind nodes whose location.file
    // matches. `location` is a stringified JSON object the parser
    // writes for every node; `json_extract` is the same primitive
    // graphengine-analysis uses elsewhere.
    let mut stmt = conn.prepare(
        "SELECT id, fqn, kind FROM nodes
         WHERE kind IN ('Function', 'Method', 'Constructor')
           AND json_extract(location, '$.file') = ?1
         ORDER BY fqn",
    )?;
    let seeds: Vec<NodeRef> = stmt
        .query_map([file_path], |row| {
            Ok(NodeRef {
                id: row.get(0)?,
                fqn: row.get(1)?,
                kind: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    if seeds.is_empty() {
        return Ok(FileBlastRadiusResult {
            file_path: file_path.to_string(),
            seeds,
            rows: Vec::new(),
            cap_hit: false,
        });
    }

    // Step 2: union per-seed upstream BFS. We keep the minimum
    // depth encountered for any (node, seed) pair so the rendered
    // "depth 1" rows are the closest callers, not the depth of the
    // last seed iterated. We also keep the *best* (highest-trust)
    // edge tier across seeds — if any seed reaches `node` via a
    // Tier 3 LSP edge, the file-aggregated row is Tier 3 even if
    // another seed only reached it heuristically.
    let seed_ids: HashSet<String> = seeds.iter().map(|s| s.id.clone()).collect();
    let mut by_id: HashMap<String, (usize, NodeRef, Vec<NodeRef>, Option<String>)> = HashMap::new();
    let mut cap_hit = false;
    for seed in &seeds {
        let rows = walk_calls(conn, &seed.id, Direction::In, max_depth, cap_per_seed)?;
        if rows.len() >= cap_per_seed {
            cap_hit = true;
        }
        for (depth, node, tier) in rows {
            if seed_ids.contains(&node.id) {
                continue;
            }
            let entry = by_id
                .entry(node.id.clone())
                .or_insert_with(|| (depth, node.clone(), Vec::new(), tier.clone()));
            if depth < entry.0 {
                entry.0 = depth;
            }
            entry.3 = best_tier(entry.3.as_deref(), tier.as_deref()).map(|s| s.to_string());
            if !entry.2.iter().any(|s| s.id == seed.id) {
                entry.2.push(seed.clone());
            }
        }
    }

    let mut rows: Vec<FileBlastRadiusRow> = by_id
        .into_values()
        .map(|(depth, node, mut via_seeds, edge_evidence_tier)| {
            via_seeds.sort_by(|a, b| a.fqn.cmp(&b.fqn));
            FileBlastRadiusRow {
                depth,
                node,
                via_seeds,
                edge_evidence_tier,
            }
        })
        .collect();
    rows.sort_by(|a, b| a.depth.cmp(&b.depth).then(a.node.fqn.cmp(&b.node.fqn)));

    Ok(FileBlastRadiusResult {
        file_path: file_path.to_string(),
        seeds,
        rows,
        cap_hit,
    })
}

// ---------------------------------------------------------------------------
// Path: BFS-based shortest call path from `from` to `to`
// ---------------------------------------------------------------------------

/// BFS shortest call-path between two function ids. Returns `Ok(None)`
/// when no Call-edge path exists in the recorded graph.
pub fn shortest_path(
    conn: &Connection,
    from_id: &str,
    to_id: &str,
    max_depth: usize,
) -> Result<Option<Vec<NodeRef>>, GraphQueryError> {
    use std::collections::{HashMap, VecDeque};

    if from_id == to_id {
        // Degenerate but reasonable: emit a single-node path.
        let mut stmt = conn.prepare("SELECT id, fqn, kind FROM nodes WHERE id = ?1")?;
        let mut rows = stmt.query([from_id])?;
        if let Some(row) = rows.next()? {
            return Ok(Some(vec![NodeRef {
                id: row.get(0)?,
                fqn: row.get(1)?,
                kind: row.get(2)?,
            }]));
        }
        return Ok(None);
    }

    let mut parent: HashMap<String, String> = HashMap::new();
    let mut queue: VecDeque<(usize, String)> = VecDeque::new();
    queue.push_back((0, from_id.to_string()));
    parent.insert(from_id.to_string(), String::new());

    let mut stmt = conn.prepare(
        "SELECT to_id FROM edges WHERE from_id = ?1 AND json_extract(kind, '$.kind') = 'Call'",
    )?;

    let mut found = false;
    while let Some((depth, id)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }
        let next_ids: Vec<String> = {
            let mut rs = stmt.query([&id])?;
            let mut acc = Vec::new();
            while let Some(r) = rs.next()? {
                acc.push(r.get(0)?);
            }
            acc
        };
        for next in next_ids {
            if parent.contains_key(&next) {
                continue;
            }
            parent.insert(next.clone(), id.clone());
            if next == to_id {
                found = true;
                break;
            }
            queue.push_back((depth + 1, next));
        }
        if found {
            break;
        }
    }
    if !found {
        return Ok(None);
    }

    let mut chain: Vec<String> = Vec::new();
    let mut cursor = to_id.to_string();
    while !cursor.is_empty() {
        chain.push(cursor.clone());
        cursor = parent.get(&cursor).cloned().unwrap_or_default();
    }
    chain.reverse();
    let placeholders = std::iter::repeat_n("?", chain.len())
        .collect::<Vec<_>>()
        .join(",");
    let mut node_stmt = conn.prepare(&format!(
        "SELECT id, fqn, kind FROM nodes WHERE id IN ({placeholders})"
    ))?;
    let params: Vec<&dyn rusqlite::ToSql> =
        chain.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    let mut rows = node_stmt.query(params.as_slice())?;
    let mut by_id: HashMap<String, NodeRef> = HashMap::new();
    while let Some(row) = rows.next()? {
        let n = NodeRef {
            id: row.get(0)?,
            fqn: row.get(1)?,
            kind: row.get(2)?,
        };
        by_id.insert(n.id.clone(), n);
    }
    let path: Vec<NodeRef> = chain
        .into_iter()
        .filter_map(|id| by_id.remove(&id))
        .collect();
    Ok(Some(path))
}

// ---------------------------------------------------------------------------
// Module coupling: how many cross-module Call edges fire between any
// two modules.
// ---------------------------------------------------------------------------

/// Returns the top coupled module pairs ordered by edge count. The
/// query uses the `Contains` edge to find each function's owning
/// module, then groups Call edges by (from_module, to_module) where
/// `from_module != to_module`.
pub fn module_coupling(
    conn: &Connection,
    limit: usize,
) -> Result<Vec<CoupledModuleRow>, GraphQueryError> {
    let mut stmt = conn.prepare(
        "WITH module_of AS (
             SELECT c.to_id AS fn_id, c.from_id AS module_id
             FROM edges c
             WHERE json_extract(c.kind, '$.kind') = 'Contains'
         )
         SELECT
             mf.module_id AS from_module,
             mt.module_id AS to_module,
             COUNT(*) AS edge_count
         FROM edges e
         JOIN module_of mf ON mf.fn_id = e.from_id
         JOIN module_of mt ON mt.fn_id = e.to_id
         WHERE json_extract(e.kind, '$.kind') = 'Call'
           AND mf.module_id != mt.module_id
         GROUP BY mf.module_id, mt.module_id
         ORDER BY edge_count DESC, from_module ASC, to_module ASC
         LIMIT ?1",
    )?;
    let pairs: Vec<(String, String, i64)> = stmt
        .query_map([limit as i64], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    if pairs.is_empty() {
        return Ok(Vec::new());
    }
    let mut all_ids: Vec<String> = pairs
        .iter()
        .flat_map(|(a, b, _)| [a.clone(), b.clone()])
        .collect();
    all_ids.sort();
    all_ids.dedup();
    let lookup = super::nodes_by_id(conn, &all_ids)?;
    Ok(pairs
        .into_iter()
        .filter_map(|(from, to, count)| {
            let from = lookup.get(&from)?.clone();
            let to = lookup.get(&to)?.clone();
            Some(CoupledModuleRow {
                from,
                to,
                edge_count: count,
            })
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Cycles: simple cycle enumeration on the Call graph
// ---------------------------------------------------------------------------

/// Returns up to `limit` simple cycles on the Call graph. The
/// algorithm is a depth-bounded DFS from every Function node; we
/// canonicalise each cycle by its smallest-id rotation so we report
/// each topological cycle exactly once. Heavy graphs cap at
/// `max_depth = 8` (matches the analysis crate's default cycle
/// length cap) to keep runtime predictable.
pub fn cycles(
    conn: &Connection,
    limit: usize,
    max_depth: usize,
) -> Result<Vec<CycleRow>, GraphQueryError> {
    use std::collections::HashMap;

    let mut nodes_stmt = conn.prepare("SELECT id FROM nodes WHERE kind = 'Function'")?;
    let function_ids: Vec<String> = nodes_stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    // Avoid the O(N^2) bomb of re-querying outgoing edges per seed by
    // pre-loading the adjacency once. On the typical scan this is a
    // few hundred KB.
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    {
        let mut all_stmt = conn.prepare(
            "SELECT from_id, to_id FROM edges WHERE json_extract(kind, '$.kind') = 'Call'",
        )?;
        let mut rows = all_stmt.query([])?;
        while let Some(row) = rows.next()? {
            let from: String = row.get(0)?;
            let to: String = row.get(1)?;
            adjacency.entry(from).or_default().push(to);
        }
    }
    let mut seen_cycles: HashSet<Vec<String>> = HashSet::new();
    let mut results: Vec<Vec<String>> = Vec::new();

    'seeds: for seed in function_ids {
        if results.len() >= limit {
            break;
        }
        // DFS with explicit stack: each frame holds (current path,
        // iterator over next callees). Recursion would also work but
        // an explicit stack is friendlier to graphs with deep call
        // chains.
        let mut stack: Vec<(Vec<String>, usize)> = vec![(vec![seed.clone()], 0)];
        while let Some((path, next_idx)) = stack.last_mut() {
            let current = path.last().cloned().unwrap();
            let nexts = adjacency.get(&current).cloned().unwrap_or_default();
            if *next_idx >= nexts.len() {
                stack.pop();
                continue;
            }
            let next = nexts[*next_idx].clone();
            *next_idx += 1;

            if path.len() > max_depth {
                continue;
            }
            if next == seed && path.len() > 1 {
                let canon = canonicalise_cycle(path);
                if seen_cycles.insert(canon.clone()) {
                    results.push(canon);
                    if results.len() >= limit {
                        break 'seeds;
                    }
                }
                continue;
            }
            if path.contains(&next) {
                continue;
            }
            let mut new_path = path.clone();
            new_path.push(next);
            stack.push((new_path, 0));
        }
    }

    let mut all_ids: Vec<String> = results.iter().flatten().cloned().collect();
    all_ids.sort();
    all_ids.dedup();
    let lookup = super::nodes_by_id(conn, &all_ids)?;
    Ok(results
        .into_iter()
        .map(|ids| {
            let members: Vec<NodeRef> = ids
                .iter()
                .filter_map(|id| lookup.get(id).cloned())
                .collect();
            CycleRow {
                length: members.len(),
                members,
            }
        })
        .collect())
}

fn canonicalise_cycle(path: &[String]) -> Vec<String> {
    if path.is_empty() {
        return Vec::new();
    }
    // Rotate so the minimal id is first; ensures `a -> b -> c -> a`
    // and `b -> c -> a -> b` are the same key.
    let min_idx = path
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.cmp(b.1))
        .map(|(i, _)| i)
        .unwrap_or(0);
    let mut rotated: Vec<String> = path[min_idx..].to_vec();
    rotated.extend_from_slice(&path[..min_idx]);
    rotated
}

// ---------------------------------------------------------------------------
// Slice: bidirectional traversal around a seed
// ---------------------------------------------------------------------------

/// "Slice" = the union of upstream and downstream Call-edge reach
/// from `seed`. Useful as a single-call answer to "everything that
/// matters near this symbol". The renderer can group by depth and
/// direction.
#[cfg(test)]
pub fn build_test_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE nodes (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            fqn TEXT NOT NULL,
            location TEXT NOT NULL DEFAULT '{}',
            provenance TEXT NOT NULL DEFAULT '{}',
            properties TEXT NOT NULL DEFAULT '{}',
            trait_metadata TEXT
        );
        CREATE TABLE edges (
            from_id TEXT NOT NULL,
            to_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            provenance TEXT NOT NULL DEFAULT '{}',
            PRIMARY KEY (from_id, to_id, kind)
        );
        CREATE INDEX idx_nodes_kind ON nodes(kind);
        CREATE INDEX idx_nodes_fqn ON nodes(fqn);
        CREATE INDEX idx_edges_from ON edges(from_id);
        CREATE INDEX idx_edges_to ON edges(to_id);

        INSERT INTO nodes (id, kind, fqn, location) VALUES
            ('m1', 'Module', 'crate::a',  '{}'),
            ('m2', 'Module', 'crate::b',  '{}'),
            ('m3', 'Module', 'crate::orphan', '{}'),
            ('fn_root',      'Function', 'crate::a::root', '{"file":"src/a.rs","start_line":1}'),
            ('fn_mid',       'Function', 'crate::a::mid',  '{"file":"src/a.rs","start_line":2}'),
            ('fn_leaf',      'Function', 'crate::a::leaf', '{"file":"src/a.rs","start_line":3}'),
            ('fn_hot',       'Function', 'crate::b::hot',  '{"file":"src/b.rs","start_line":1}'),
            ('fn_orphan',    'Function', 'crate::orphan::lonely', '{"file":"src/orphan.rs"}'),
            ('fn_caller_a',  'Function', 'crate::a::ca',   '{"file":"src/a.rs","start_line":5}'),
            ('fn_caller_b',  'Function', 'crate::a::cb',   '{"file":"src/a.rs","start_line":6}');

        -- Contains edges: module → function
        INSERT INTO edges (from_id, to_id, kind) VALUES
            ('m1', 'fn_root',      '{"kind":"Contains"}'),
            ('m1', 'fn_mid',       '{"kind":"Contains"}'),
            ('m1', 'fn_leaf',      '{"kind":"Contains"}'),
            ('m1', 'fn_caller_a',  '{"kind":"Contains"}'),
            ('m1', 'fn_caller_b',  '{"kind":"Contains"}'),
            ('m2', 'fn_hot',       '{"kind":"Contains"}'),
            ('m3', 'fn_orphan',    '{"kind":"Contains"}');

        -- Call graph:
        -- root → mid → leaf
        -- root → hot
        -- mid → hot
        -- caller_a → hot
        -- caller_b → hot
        -- (creates a 2-node SCC: leaf → mid OFF for now, but add one cycle)
        --
        -- Provenance choices exercise every tier so the
        -- edge_evidence_tier annotation has something to map: root→mid
        -- is LSP-verified (tier_3), root→hot is tree-sitter (tier_0),
        -- caller_b→hot is heuristic (tier_1), and the leaf→root cycle
        -- edge has an empty `{}` provenance to exercise the
        -- legacy/unknown fall-through.
        INSERT INTO edges (from_id, to_id, kind, provenance) VALUES
            ('fn_root', 'fn_mid',  '{"kind":"Call"}', '{"source":"Lsp","confidence":"High"}'),
            ('fn_mid',  'fn_leaf', '{"kind":"Call"}', '{"source":"TreeSitter","confidence":"High"}'),
            ('fn_root', 'fn_hot',  '{"kind":"Call"}', '{"source":"TreeSitter","confidence":"High"}'),
            ('fn_mid',  'fn_hot',  '{"kind":"Call"}', '{"source":"Lsp","confidence":"High"}'),
            ('fn_caller_a', 'fn_hot', '{"kind":"Call"}', '{"source":"TreeSitter","confidence":"High"}'),
            ('fn_caller_b', 'fn_hot', '{"kind":"Call"}', '{"source":"Heuristic","confidence":"Low"}'),
            -- intentional cycle: leaf → root, with intentionally
            -- empty provenance to pin the unknown-tier fallback.
            ('fn_leaf', 'fn_root', '{"kind":"Call"}', '{}');
        "#,
    )
    .unwrap();
    conn
}

pub fn slice(
    conn: &Connection,
    seed_id: &str,
    depth: usize,
    cap: usize,
) -> Result<Vec<SliceNode>, GraphQueryError> {
    let upstream = walk_calls(conn, seed_id, Direction::In, depth, cap)?;
    let downstream = walk_calls(conn, seed_id, Direction::Out, depth, cap)?;
    let mut out: Vec<SliceNode> = Vec::with_capacity(upstream.len() + downstream.len());
    for (d, n, tier) in upstream {
        out.push(SliceNode {
            depth: d,
            direction: SliceDirection::Upstream,
            node: n,
            edge_evidence_tier: tier,
        });
    }
    for (d, n, tier) in downstream {
        out.push(SliceNode {
            depth: d,
            direction: SliceDirection::Downstream,
            node: n,
            edge_evidence_tier: tier,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fan_in_ranks_by_call_count() {
        let conn = build_test_db();
        let rows = top_fan_in(&conn, 5).unwrap();
        assert!(rows.iter().any(|r| r.node.fqn.ends_with("::hot")));
        // hot has 4 callers, more than anything else in the fixture
        let hot = rows.iter().find(|r| r.node.fqn.ends_with("::hot")).unwrap();
        assert_eq!(hot.count, 4);
    }

    #[test]
    fn fan_out_ranks_by_call_count() {
        let conn = build_test_db();
        let rows = top_fan_out(&conn, 5).unwrap();
        // root calls mid + hot = 2 callees
        let root = rows
            .iter()
            .find(|r| r.node.fqn.ends_with("::root"))
            .unwrap();
        assert_eq!(root.count, 2);
    }

    #[test]
    fn hotspots_honours_threshold() {
        let conn = build_test_db();
        let none = hotspots(&conn, 100, 5).unwrap();
        assert!(none.is_empty(), "no node has >=100 callers");
        let some = hotspots(&conn, 4, 5).unwrap();
        assert_eq!(some.len(), 1);
    }

    #[test]
    fn callers_and_callees_match_edges() {
        let conn = build_test_db();
        let callers_of_hot = callers(&conn, "fn_hot").unwrap();
        assert_eq!(callers_of_hot.len(), 4);
        let callees_of_root = callees(&conn, "fn_root").unwrap();
        assert_eq!(callees_of_root.len(), 2);
    }

    #[test]
    fn blast_radius_returns_upstream_dependents_with_depth_cap() {
        let conn = build_test_db();
        // fn_hot has exactly four direct callers in the fixture
        // (root, mid, caller_a, caller_b). At depth 1, blast_radius
        // must return exactly those four — the upstream/reverse
        // direction is the whole point of blast radius.
        let d1 = blast_radius(&conn, "fn_hot", 1, 50).unwrap();
        let d1_ids: std::collections::HashSet<_> = d1.iter().map(|n| n.node.id.as_str()).collect();
        assert_eq!(
            d1_ids,
            ["fn_root", "fn_mid", "fn_caller_a", "fn_caller_b"]
                .into_iter()
                .collect()
        );
        // At depth 3 the cycle (leaf → root) pulls fn_leaf into the
        // upstream closure of fn_hot as well.
        let d3 = blast_radius(&conn, "fn_hot", 3, 50).unwrap();
        assert!(d3.iter().any(|n| n.node.id == "fn_leaf"));
    }

    /// Regression: a previous revision walked the forward direction
    /// (callees), inverting the meaning of every MCP "blast radius"
    /// answer. fn_root's direct *callees* are {fn_mid, fn_hot}; its
    /// only direct *caller* is fn_leaf (via the cycle edge). If this
    /// test ever sees mid or hot in a depth-1 result for fn_root,
    /// the direction has silently flipped back to wrong.
    #[test]
    fn blast_radius_walks_callers_not_callees() {
        let conn = build_test_db();
        let d1 = blast_radius(&conn, "fn_root", 1, 50).unwrap();
        let d1_ids: std::collections::HashSet<_> = d1.iter().map(|n| n.node.id.as_str()).collect();
        assert!(
            d1_ids.contains("fn_leaf"),
            "expected upstream caller fn_leaf at depth 1 of fn_root, got {d1_ids:?}"
        );
        assert!(
            !d1_ids.contains("fn_mid") && !d1_ids.contains("fn_hot"),
            "blast_radius walked callees instead of callers: {d1_ids:?}"
        );
        // And every row must be labeled as upstream.
        assert!(
            d1.iter()
                .all(|n| matches!(n.direction, SliceDirection::Upstream)),
            "expected every row tagged SliceDirection::Upstream"
        );
    }

    #[test]
    fn shortest_path_finds_known_chain() {
        let conn = build_test_db();
        let path = shortest_path(&conn, "fn_root", "fn_leaf", 5).unwrap();
        let path = path.expect("should find a path");
        assert_eq!(path.len(), 3);
        assert_eq!(path.first().unwrap().id, "fn_root");
        assert_eq!(path.last().unwrap().id, "fn_leaf");
    }

    #[test]
    fn shortest_path_handles_no_path() {
        let conn = build_test_db();
        let path = shortest_path(&conn, "fn_root", "fn_orphan", 5).unwrap();
        assert!(path.is_none());
    }

    #[test]
    fn dead_code_lists_nodes_without_callers() {
        let conn = build_test_db();
        let rows = dead_code(&conn, 20).unwrap();
        assert!(rows.iter().any(|r| r.node.id == "fn_orphan"));
        // root has no callers either (top of chain) so it should show up too
        assert!(
            rows.iter().any(|r| r.node.id == "fn_root")
                || rows.iter().any(|r| r.node.id == "fn_caller_a")
        );
    }

    #[test]
    fn module_coupling_groups_by_owning_module() {
        let conn = build_test_db();
        let rows = module_coupling(&conn, 10).unwrap();
        // m1 -> m2 should be top: root→hot, mid→hot, caller_a→hot, caller_b→hot = 4 edges
        assert!(!rows.is_empty());
        let top = &rows[0];
        assert_eq!(top.from.id, "m1");
        assert_eq!(top.to.id, "m2");
        assert_eq!(top.edge_count, 4);
    }

    #[test]
    fn cycles_finds_known_triangle() {
        let conn = build_test_db();
        let rows = cycles(&conn, 5, 6).unwrap();
        // root → mid → leaf → root is a real cycle
        assert!(rows.iter().any(|c| c.length == 3));
    }

    #[test]
    fn file_blast_radius_unions_per_seed_upstream_with_attribution() {
        let conn = build_test_db();
        // src/b.rs contains fn_hot. Its upstream callers in the
        // fixture are fn_root, fn_mid, fn_caller_a, fn_caller_b (all
        // in src/a.rs). The file blast radius for src/b.rs should
        // surface those four with depth 1 and attribute every row to
        // its sole seed (fn_hot).
        let result = file_blast_radius(&conn, "src/b.rs", 3, 50).unwrap();
        assert_eq!(result.file_path, "src/b.rs");
        assert_eq!(result.seeds.len(), 1, "src/b.rs has one function");
        assert_eq!(result.seeds[0].id, "fn_hot");
        let row_ids: std::collections::HashSet<&str> =
            result.rows.iter().map(|r| r.node.id.as_str()).collect();
        assert!(row_ids.contains("fn_root"));
        assert!(row_ids.contains("fn_mid"));
        assert!(row_ids.contains("fn_caller_a"));
        assert!(row_ids.contains("fn_caller_b"));
        assert!(
            !row_ids.contains("fn_hot"),
            "seed must not appear in rows (rows are the EXTERNAL blast radius)"
        );
        for row in &result.rows {
            assert_eq!(row.via_seeds.len(), 1, "fn_hot is the only seed");
            assert_eq!(row.via_seeds[0].id, "fn_hot");
        }
    }

    #[test]
    fn file_blast_radius_attributes_to_multiple_seeds_when_shared() {
        // src/a.rs contains fn_root, fn_mid, fn_leaf, fn_caller_a,
        // fn_caller_b. Since they're all in the same file, the
        // *external* blast radius (excluding intra-file edges) is
        // empty — every Call edge in the fixture points either at
        // another src/a.rs symbol or at fn_hot (which is a callee,
        // not a caller). The expected upstream callers from outside
        // src/a.rs is therefore zero rows; this test pins that empty
        // result, plus exercises the "no external callers" path.
        let conn = build_test_db();
        let result = file_blast_radius(&conn, "src/a.rs", 3, 50).unwrap();
        assert_eq!(result.seeds.len(), 5);
        assert!(
            result.rows.is_empty(),
            "src/a.rs has no upstream callers outside itself in the fixture; got: {:?}",
            result.rows.iter().map(|r| &r.node.fqn).collect::<Vec<_>>()
        );
    }

    #[test]
    fn file_blast_radius_empty_when_file_has_no_function_nodes() {
        let conn = build_test_db();
        let result = file_blast_radius(&conn, "src/does_not_exist.rs", 3, 50).unwrap();
        assert!(result.seeds.is_empty());
        assert!(result.rows.is_empty());
        assert!(!result.cap_hit);
    }

    #[test]
    fn blast_radius_annotates_each_hop_with_edge_tier() {
        // Pins the per-hop tier contract from Q2: agents need to
        // know which BFS rows are LSP-verified vs heuristic so they
        // can downgrade their narrative for low-trust hops.
        let conn = build_test_db();
        let rows = blast_radius(&conn, "fn_hot", 1, 50).unwrap();
        let by_id: std::collections::HashMap<&str, &Option<String>> = rows
            .iter()
            .map(|r| (r.node.id.as_str(), &r.edge_evidence_tier))
            .collect();
        assert_eq!(
            by_id.get("fn_root").unwrap().as_deref(),
            Some("tier_0"),
            "root→hot fixture edge is TreeSitter"
        );
        assert_eq!(
            by_id.get("fn_mid").unwrap().as_deref(),
            Some("tier_3"),
            "mid→hot fixture edge is LSP"
        );
        assert_eq!(
            by_id.get("fn_caller_b").unwrap().as_deref(),
            Some("tier_1"),
            "caller_b→hot fixture edge is Heuristic"
        );
    }

    #[test]
    fn blast_radius_reports_unknown_tier_when_provenance_empty() {
        // The leaf→root edge in the fixture has '{}' provenance to
        // simulate legacy scans that pre-date the provenance
        // column. The BFS must keep the row but report None for the
        // tier rather than fabricating one.
        let conn = build_test_db();
        let rows = blast_radius(&conn, "fn_root", 1, 50).unwrap();
        let leaf = rows
            .iter()
            .find(|r| r.node.id == "fn_leaf")
            .expect("leaf must appear at depth 1");
        assert!(
            leaf.edge_evidence_tier.is_none(),
            "expected None for empty-provenance edge, got {:?}",
            leaf.edge_evidence_tier
        );
    }

    #[test]
    fn file_blast_radius_aggregates_best_tier_across_seeds() {
        // src/b.rs has fn_hot. Multiple paths upstream of fn_hot
        // have different tiers; the aggregated row for fn_caller_b
        // (which has a Tier 1 Heuristic edge into fn_hot) should
        // report tier_1, while fn_root (Tier 0 TreeSitter edge into
        // fn_hot) reports tier_0.
        let conn = build_test_db();
        let result = file_blast_radius(&conn, "src/b.rs", 3, 50).unwrap();
        let tier_for = |id: &str| -> Option<String> {
            result
                .rows
                .iter()
                .find(|r| r.node.id == id)
                .and_then(|r| r.edge_evidence_tier.clone())
        };
        assert_eq!(tier_for("fn_root").as_deref(), Some("tier_0"));
        assert_eq!(tier_for("fn_caller_b").as_deref(), Some("tier_1"));
        assert_eq!(tier_for("fn_mid").as_deref(), Some("tier_3"));
    }

    #[test]
    fn slice_unions_upstream_and_downstream() {
        let conn = build_test_db();
        let rows = slice(&conn, "fn_mid", 2, 50).unwrap();
        let has_upstream = rows
            .iter()
            .any(|n| matches!(n.direction, SliceDirection::Upstream));
        let has_downstream = rows
            .iter()
            .any(|n| matches!(n.direction, SliceDirection::Downstream));
        assert!(has_upstream && has_downstream);
    }
}
