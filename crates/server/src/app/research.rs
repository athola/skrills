//! Research tool handlers for the tome crate.
//!
//! Implements MCP tool handlers for academic research API orchestration,
//! knowledge graph management, citation tracking, and TRIZ contradiction resolution.

use anyhow::{anyhow, Result};
use rmcp::model::{CallToolResult, Content};
use serde_json::{json, Map as JsonMap, Value};

use crate::app::SkillService;

/// Resolve the skrills-tome cache directory.
///
/// Mirrors the logic in `ResearchCache::open()` so that the knowledge graph
/// and citation databases land alongside the API cache.
fn tome_cache_dir() -> Result<std::path::PathBuf> {
    let base = dirs::cache_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join(".cache")))
        .ok_or_else(|| anyhow!("cannot determine cache directory: HOME is unset"))?;
    let dir = base.join("skrills-tome");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

impl SkillService {
    // --- #168: Research API Tools ---

    pub(crate) async fn search_papers_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
        use skrills_tome::clients::arxiv::ArxivClient;
        use skrills_tome::clients::openalex::OpenAlexClient;
        use skrills_tome::clients::semantic_scholar::SemanticScholarClient;
        use skrills_tome::models::Paper;
        use std::collections::HashSet;

        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing required parameter: query"))?;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(10)
            .min(100) as usize;
        let sources: Vec<String> = args
            .get("sources")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_else(|| {
                vec![
                    "arxiv".to_string(),
                    "semantic_scholar".to_string(),
                    "openalex".to_string(),
                ]
            });

        let mut all_papers: Vec<Paper> = Vec::new();
        let mut errors: Vec<String> = Vec::new();

        for source in &sources {
            let result: Result<Vec<Paper>, _> = match source.as_str() {
                "arxiv" => ArxivClient::new().search(query, limit).await,
                "semantic_scholar" => SemanticScholarClient::new().search(query, limit).await,
                "openalex" => OpenAlexClient::new().search(query, limit).await,
                other => {
                    errors.push(format!("Unknown source: {other}"));
                    continue;
                }
            };
            match result {
                Ok(papers) => all_papers.extend(papers),
                Err(e) => errors.push(format!("{source}: {e}")),
            }
        }

        // Deduplicate by DOI
        let mut seen_dois = HashSet::new();
        let mut deduped: Vec<Paper> = Vec::new();
        for paper in all_papers {
            if let Some(doi) = &paper.doi {
                if !seen_dois.insert(doi.clone()) {
                    continue;
                }
            }
            deduped.push(paper);
        }
        deduped.truncate(limit);

        let paper_json: Vec<Value> = deduped
            .iter()
            .map(|p| {
                json!({
                    "id": p.id,
                    "title": p.title,
                    "authors": p.authors,
                    "abstract": p.abstract_text,
                    "year": p.year,
                    "doi": p.doi,
                    "url": p.url,
                    "source": p.source,
                    "citation_count": p.citation_count,
                    "pdf_url": p.pdf_url,
                })
            })
            .collect();

        let mut text = format!("Found {} papers", deduped.len());
        if !errors.is_empty() {
            text.push_str(&format!(" ({} source errors)", errors.len()));
        }

        Ok(CallToolResult {
            content: vec![Content::text(text)],
            structured_content: Some(json!({
                "papers": paper_json,
                "count": deduped.len(),
                "errors": errors,
            })),
            is_error: Some(false),
            meta: None,
        })
    }

    pub(crate) async fn search_discussions_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
        use skrills_tome::clients::hn_algolia::HnAlgoliaClient;

        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing required parameter: query"))?;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

        let client = HnAlgoliaClient::new();
        let discussions = client.search(query, limit).await?;

        let discussion_json: Vec<Value> = discussions
            .iter()
            .map(|d| {
                json!({
                    "id": d.id,
                    "title": d.title,
                    "url": d.url,
                    "points": d.points,
                    "comment_count": d.comment_count,
                    "source": d.source,
                    "created_at": d.created_at.map(|t| {
                        t.format(&time::format_description::well_known::Rfc3339)
                            .unwrap_or_default()
                    }),
                })
            })
            .collect();

        Ok(CallToolResult {
            content: vec![Content::text(format!(
                "Found {} discussions",
                discussions.len()
            ))],
            structured_content: Some(json!({
                "discussions": discussion_json,
                "count": discussions.len(),
            })),
            is_error: Some(false),
            meta: None,
        })
    }

    pub(crate) async fn resolve_doi_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
        use skrills_tome::clients::crossref::CrossRefClient;
        use skrills_tome::clients::unpaywall::UnpaywallClient;

        let doi = args
            .get("doi")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing required parameter: doi"))?;

        let crossref = CrossRefClient::new();
        let metadata = crossref.resolve_doi(doi).await?;

        let unpaywall = UnpaywallClient::default();
        let pdf_url = unpaywall.find_pdf_url(doi).await.unwrap_or(None);

        Ok(CallToolResult {
            content: vec![Content::text(format!(
                "{} ({})",
                metadata.title,
                metadata.year.map(|y| y.to_string()).unwrap_or_default()
            ))],
            structured_content: Some(json!({
                "doi": metadata.doi,
                "title": metadata.title,
                "authors": metadata.authors,
                "publisher": metadata.publisher,
                "year": metadata.year,
                "url": metadata.url,
                "journal": metadata.journal,
                "pdf_url": pdf_url,
            })),
            is_error: Some(false),
            meta: None,
        })
    }

    pub(crate) async fn fetch_pdf_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
        use skrills_tome::cache::ResearchCache;
        use skrills_tome::clients::unpaywall::UnpaywallClient;

        let doi = args
            .get("doi")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing required parameter: doi"))?;

        let unpaywall = UnpaywallClient::default();
        let pdf_url = unpaywall
            .find_pdf_url(doi)
            .await?
            .ok_or_else(|| anyhow!("No open-access PDF found for DOI: {doi}"))?;

        let cache = ResearchCache::open()?;
        let pdf_path = cache
            .pdf_dir()
            .join(format!("{}.pdf", doi.replace('/', "_")));

        // Download if not already cached
        if !pdf_path.exists() {
            let client = reqwest::Client::new();
            let bytes = client.get(&pdf_url).send().await?.bytes().await?;
            std::fs::write(&pdf_path, &bytes)?;
        }

        let path_str = pdf_path.to_string_lossy().to_string();

        Ok(CallToolResult {
            content: vec![Content::text(format!("PDF cached at: {path_str}"))],
            structured_content: Some(json!({
                "path": path_str,
                "doi": doi,
                "url": pdf_url,
                "cached": true,
            })),
            is_error: Some(false),
            meta: None,
        })
    }

    // --- #169: Advanced Research Tools ---

    pub(crate) fn query_knowledge_graph_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
        use skrills_tome::knowledge_graph::{KnowledgeGraph, NodeKind};

        let db_path = tome_cache_dir()?.join("knowledge.db");
        let kg = KnowledgeGraph::open(&db_path)?;

        if let Some(node_id) = args.get("node_id").and_then(|v| v.as_str()) {
            let direction = args
                .get("direction")
                .and_then(|v| v.as_str())
                .unwrap_or("both");

            let node = kg.get_node(node_id)?;
            let mut edges_from = Vec::new();
            let mut edges_to = Vec::new();

            if direction == "from" || direction == "both" {
                edges_from = kg.edges_from(node_id)?;
            }
            if direction == "to" || direction == "both" {
                edges_to = kg.edges_to(node_id)?;
            }

            Ok(CallToolResult {
                content: vec![Content::text(format!(
                    "Node {}: {} outgoing, {} incoming edges",
                    node_id,
                    edges_from.len(),
                    edges_to.len()
                ))],
                structured_content: Some(json!({
                    "node": node.map(|n| json!({
                        "id": n.id,
                        "kind": n.kind.as_str(),
                        "label": n.label,
                    })),
                    "edges_from": edges_from.iter().map(|e| json!({
                        "target": e.target_id,
                        "kind": e.kind.as_str(),
                        "weight": e.weight,
                    })).collect::<Vec<_>>(),
                    "edges_to": edges_to.iter().map(|e| json!({
                        "source": e.source_id,
                        "kind": e.kind.as_str(),
                        "weight": e.weight,
                    })).collect::<Vec<_>>(),
                })),
                is_error: Some(false),
                meta: None,
            })
        } else if let Some(query) = args.get("query").and_then(|v| v.as_str()) {
            let kind = args
                .get("kind")
                .and_then(|v| v.as_str())
                .and_then(|k| match k {
                    "topic" => Some(NodeKind::Topic),
                    "paper" => Some(NodeKind::Paper),
                    "implementation" => Some(NodeKind::Implementation),
                    "discussion" => Some(NodeKind::Discussion),
                    _ => None,
                });

            let nodes = kg.search_nodes(query, kind)?;
            let node_json: Vec<Value> = nodes
                .iter()
                .map(|n| {
                    json!({
                        "id": n.id,
                        "kind": n.kind.as_str(),
                        "label": n.label,
                    })
                })
                .collect();

            Ok(CallToolResult {
                content: vec![Content::text(format!("Found {} nodes", nodes.len()))],
                structured_content: Some(json!({ "nodes": node_json, "count": nodes.len() })),
                is_error: Some(false),
                meta: None,
            })
        } else {
            let (node_count, edge_count) = kg.stats()?;
            Ok(CallToolResult {
                content: vec![Content::text(format!(
                    "Knowledge graph: {} nodes, {} edges",
                    node_count, edge_count
                ))],
                structured_content: Some(json!({
                    "node_count": node_count,
                    "edge_count": edge_count,
                })),
                is_error: Some(false),
                meta: None,
            })
        }
    }

    pub(crate) fn add_knowledge_node_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
        use skrills_tome::knowledge_graph::{KnowledgeGraph, NodeKind};

        let id = args
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing required parameter: id"))?;
        let kind_str = args
            .get("kind")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing required parameter: kind"))?;
        let label = args
            .get("label")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing required parameter: label"))?;
        let metadata = args.get("metadata").map(|v| v.to_string());

        let kind = match kind_str {
            "topic" => NodeKind::Topic,
            "paper" => NodeKind::Paper,
            "implementation" => NodeKind::Implementation,
            "discussion" => NodeKind::Discussion,
            other => return Err(anyhow!("Unknown node kind: {other}")),
        };

        let db_path = tome_cache_dir()?.join("knowledge.db");
        let kg = KnowledgeGraph::open(&db_path)?;
        kg.add_node(id, kind, label, metadata.as_deref())?;

        Ok(CallToolResult {
            content: vec![Content::text(format!(
                "Added node '{id}' ({kind_str}): {label}"
            ))],
            structured_content: Some(json!({"id": id, "kind": kind_str, "label": label})),
            is_error: Some(false),
            meta: None,
        })
    }

    pub(crate) fn link_knowledge_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
        use skrills_tome::knowledge_graph::{EdgeKind, KnowledgeGraph};

        let source_id = args
            .get("source_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing required parameter: source_id"))?;
        let target_id = args
            .get("target_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing required parameter: target_id"))?;
        let kind_str = args
            .get("kind")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing required parameter: kind"))?;
        let weight = args.get("weight").and_then(|v| v.as_f64()).unwrap_or(1.0);
        let metadata = args.get("metadata").map(|v| v.to_string());

        let kind = match kind_str {
            "cites" => EdgeKind::Cites,
            "implements" => EdgeKind::Implements,
            "contradicts" => EdgeKind::Contradicts,
            "extends" => EdgeKind::Extends,
            "analogous_to" => EdgeKind::AnalogousTo,
            other => return Err(anyhow!("Unknown edge kind: {other}")),
        };

        let db_path = tome_cache_dir()?.join("knowledge.db");
        let kg = KnowledgeGraph::open(&db_path)?;
        kg.add_edge(source_id, target_id, kind, weight, metadata.as_deref())?;

        Ok(CallToolResult {
            content: vec![Content::text(format!(
                "Linked {source_id} --{kind_str}--> {target_id}"
            ))],
            structured_content: Some(json!({
                "source_id": source_id,
                "target_id": target_id,
                "kind": kind_str,
                "weight": weight,
            })),
            is_error: Some(false),
            meta: None,
        })
    }

    pub(crate) fn track_citations_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
        use skrills_tome::citations::CitationTracker;
        use skrills_tome::models::{Paper, PaperSource};

        let paper_id = args
            .get("paper_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing required parameter: paper_id"))?;
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("track");

        let db_path = tome_cache_dir()?.join("citations.db");
        let tracker = CitationTracker::open(&db_path)?;

        match action {
            "track" => {
                let title = args.get("title").and_then(|v| v.as_str()).ok_or_else(|| {
                    anyhow!("Missing required parameter: title (for track action)")
                })?;
                let doi = args.get("doi").and_then(|v| v.as_str()).map(String::from);

                let paper = Paper {
                    id: paper_id.to_string(),
                    title: title.to_string(),
                    authors: Vec::new(),
                    abstract_text: None,
                    year: None,
                    doi,
                    url: None,
                    source: PaperSource::CrossRef,
                    citation_count: None,
                    pdf_url: None,
                };
                tracker.track_paper(&paper)?;

                Ok(CallToolResult {
                    content: vec![Content::text(format!("Now tracking: {title}"))],
                    structured_content: Some(
                        json!({"paper_id": paper_id, "title": title, "action": "tracked"}),
                    ),
                    is_error: Some(false),
                    meta: None,
                })
            }
            "forward" => {
                let citations = tracker.forward_citations(paper_id)?;
                let citation_json: Vec<Value> = citations
                    .iter()
                    .map(|c| {
                        json!({
                            "citing_id": c.citing_paper_id,
                            "cited_id": c.cited_paper_id,
                            "context": c.context,
                        })
                    })
                    .collect();

                Ok(CallToolResult {
                    content: vec![Content::text(format!(
                        "{} forward citations",
                        citations.len()
                    ))],
                    structured_content: Some(json!({
                        "citations": citation_json,
                        "count": citations.len(),
                        "direction": "forward",
                    })),
                    is_error: Some(false),
                    meta: None,
                })
            }
            "backward" => {
                let citations = tracker.backward_citations(paper_id)?;
                let citation_json: Vec<Value> = citations
                    .iter()
                    .map(|c| {
                        json!({
                            "citing_id": c.citing_paper_id,
                            "cited_id": c.cited_paper_id,
                            "context": c.context,
                        })
                    })
                    .collect();

                Ok(CallToolResult {
                    content: vec![Content::text(format!(
                        "{} backward citations",
                        citations.len()
                    ))],
                    structured_content: Some(json!({
                        "citations": citation_json,
                        "count": citations.len(),
                        "direction": "backward",
                    })),
                    is_error: Some(false),
                    meta: None,
                })
            }
            other => Err(anyhow!(
                "Unknown action: {other}. Use 'track', 'forward', or 'backward'"
            )),
        }
    }

    pub(crate) fn resolve_contradiction_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
        use skrills_tome::triz::TrizMatrix;

        let improve_str = args
            .get("improve")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing required parameter: improve"))?;
        let degrades_str = args
            .get("degrades")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing required parameter: degrades"))?;

        let improve = parse_parameter(improve_str)?;
        let degrades = parse_parameter(degrades_str)?;

        let matrix = TrizMatrix::new();
        let principles = matrix.resolve(improve, degrades);

        let principle_json: Vec<Value> = principles
            .iter()
            .map(|p| {
                json!({
                    "number": p.number,
                    "name": p.name,
                    "description": p.description,
                    "software_examples": p.software_examples,
                })
            })
            .collect();

        Ok(CallToolResult {
            content: vec![Content::text(format!(
                "Improving {} vs degrading {}: {} applicable principles",
                improve_str,
                degrades_str,
                principles.len()
            ))],
            structured_content: Some(json!({
                "improve": improve_str,
                "degrades": degrades_str,
                "principles": principle_json,
                "count": principles.len(),
            })),
            is_error: Some(false),
            meta: None,
        })
    }
}

fn parse_parameter(s: &str) -> Result<skrills_tome::triz::Parameter> {
    use skrills_tome::triz::Parameter;
    match s {
        "performance" => Ok(Parameter::Performance),
        "reliability" => Ok(Parameter::Reliability),
        "maintainability" => Ok(Parameter::Maintainability),
        "scalability" => Ok(Parameter::Scalability),
        "security" => Ok(Parameter::Security),
        "usability" => Ok(Parameter::Usability),
        "testability" => Ok(Parameter::Testability),
        "deployability" => Ok(Parameter::Deployability),
        "cost_efficiency" => Ok(Parameter::CostEfficiency),
        "development_speed" => Ok(Parameter::DevelopmentSpeed),
        "code_complexity" => Ok(Parameter::CodeComplexity),
        "memory_usage" => Ok(Parameter::MemoryUsage),
        "latency" => Ok(Parameter::Latency),
        "throughput" => Ok(Parameter::Throughput),
        "availability" => Ok(Parameter::Availability),
        other => Err(anyhow!("Unknown parameter: {other}")),
    }
}
