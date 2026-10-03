//! Research tool handlers for the tome crate.
//!
//! Implements MCP tool handlers for academic research API orchestration,
//! knowledge graph management, citation tracking, and TRIZ contradiction resolution.

use std::collections::HashSet;

use anyhow::{anyhow, Result};
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::{json, Map as JsonMap, Value};

use skrills_tome::cache::ResearchCache;
use skrills_tome::citations::CitationTracker;
use skrills_tome::clients::{
    arxiv::ArxivClient, crossref::CrossRefClient, hn_algolia::HnAlgoliaClient,
    openalex::OpenAlexClient, semantic_scholar::SemanticScholarClient, unpaywall::UnpaywallClient,
};
use skrills_tome::knowledge_graph::{EdgeKind, KnowledgeGraph, NodeKind};
use skrills_tome::models::{Paper, PaperSource};
use skrills_tome::triz::{Parameter, TrizMatrix};

use crate::app::SkillService;
use crate::mcp_result::{tool_err, tool_ok};

/// Largest PDF `fetch-pdf` will download.
const MAX_PDF_BYTES: u64 = 100 * 1024 * 1024;

/// Redirect hops `fetch-pdf` will follow; each hop is re-checked.
const MAX_PDF_REDIRECTS: usize = 5;

/// Cache file name for a DOI. Percent-encodes everything outside
/// `[A-Za-z0-9._-]`, so distinct DOIs never share a file (`10.1/a_b` and
/// `10.1/a/b` used to) and no DOI can name a path.
pub(crate) fn pdf_cache_file_name(doi: &str) -> String {
    let mut name = String::with_capacity(doi.len() + 4);
    for byte in doi.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-') {
            name.push(byte as char);
        } else {
            name.push_str(&format!("%{byte:02X}"));
        }
    }
    name.push_str(".pdf");
    name
}

/// Refuse a PDF location that is not plain https on a public host.
///
/// The URL comes from a third-party API and may redirect, so every hop is
/// checked. Literal loopback, private, link-local and similar addresses and
/// `localhost` names are rejected; a public name that resolves to a private
/// address is not caught here.
pub(crate) fn check_pdf_url(url: &reqwest::Url) -> Result<()> {
    use std::net::IpAddr;

    if url.scheme() != "https" {
        return Err(anyhow!("refusing non-https PDF URL: {url}"));
    }
    let blocked_ip = |ip: IpAddr| match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                // 100.64.0.0/10, carrier-grade NAT
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64)
        }
        IpAddr::V6(v6) => {
            let seg0 = v6.segments()[0];
            v6.is_loopback()
                || v6.is_unspecified()
                || (seg0 & 0xFE00) == 0xFC00 // unique local
                || (seg0 & 0xFFC0) == 0xFE80 // link local
                || v6.to_ipv4_mapped().is_some_and(|v4| {
                    v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
                })
        }
    };
    let blocked = match url.host_str() {
        None => true,
        Some(host) => {
            let bare = host.trim_start_matches('[').trim_end_matches(']');
            match bare.parse::<IpAddr>() {
                Ok(ip) => blocked_ip(ip),
                Err(_) => {
                    let name = host.trim_end_matches('.').to_ascii_lowercase();
                    name == "localhost" || name.ends_with(".localhost")
                }
            }
        }
    };
    if blocked {
        return Err(anyhow!(
            "refusing PDF URL on a local or private host: {url}"
        ));
    }
    Ok(())
}

/// Whether `path` exists and starts with the PDF signature.
fn has_pdf_magic(path: &std::path::Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 5];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .map(|()| &head == b"%PDF-")
        .unwrap_or(false)
}

/// Download `url` to `dest`: at most `max_bytes`, must start with `%PDF-`,
/// written to a temporary file in the same directory and renamed into place
/// so an interrupted download never leaves a truncated cache entry.
pub(crate) async fn download_pdf(
    client: &reqwest::Client,
    url: reqwest::Url,
    dest: &std::path::Path,
    max_bytes: u64,
) -> Result<u64> {
    use std::io::Write;

    let mut resp = client.get(url.clone()).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!(
            "PDF download failed with HTTP {}: {}",
            resp.status().as_u16(),
            url
        ));
    }
    if resp.content_length().is_some_and(|len| len > max_bytes) {
        return Err(anyhow!("PDF at {url} is larger than {max_bytes} bytes"));
    }

    let dir = dest
        .parent()
        .ok_or_else(|| anyhow!("PDF cache path has no parent: {}", dest.display()))?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    let mut written: u64 = 0;
    let mut head = Vec::with_capacity(5);
    while let Some(chunk) = resp.chunk().await? {
        written += chunk.len() as u64;
        if written > max_bytes {
            return Err(anyhow!("PDF at {url} is larger than {max_bytes} bytes"));
        }
        if head.len() < 5 {
            let need = 5 - head.len();
            head.extend_from_slice(&chunk[..need.min(chunk.len())]);
        }
        tmp.write_all(&chunk)?;
    }
    if head != b"%PDF-" {
        return Err(anyhow!("{url} did not return a PDF"));
    }
    tmp.as_file().sync_all()?;
    tmp.persist(dest).map_err(|e| e.error)?;
    Ok(written)
}

/// Resolve the skrills-tome cache directory.
fn tome_cache_dir() -> Result<std::path::PathBuf> {
    Ok(ResearchCache::cache_dir()?)
}

impl SkillService {
    // --- #168: Research API Tools ---

    pub(crate) async fn search_papers_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
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

        let all_failed = deduped.is_empty() && !errors.is_empty();
        let mut text = format!("Found {} papers", deduped.len());
        if !errors.is_empty() {
            text.push_str(&format!(" ({} source errors)", errors.len()));
        }
        if all_failed {
            text.push_str(": all sources failed");
        }

        let content = vec![ContentBlock::text(text)];
        let structured = Some(json!({
            "papers": paper_json,
            "count": deduped.len(),
            "errors": errors,
        }));
        Ok(if all_failed {
            tool_err(content, structured)
        } else {
            tool_ok(content, structured)
        })
    }

    pub(crate) async fn search_discussions_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing required parameter: query"))?;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(10)
            .min(100) as usize;

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

        Ok(tool_ok(
            vec![ContentBlock::text(format!(
                "Found {} discussions",
                discussions.len()
            ))],
            Some(json!({
                "discussions": discussion_json,
                "count": discussions.len(),
            })),
        ))
    }

    pub(crate) async fn resolve_doi_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
        let doi = args
            .get("doi")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing required parameter: doi"))?;

        let crossref = CrossRefClient::new();
        let metadata = crossref.resolve_doi(doi).await?;

        let unpaywall = UnpaywallClient::default();
        let pdf_url = match unpaywall.find_pdf_url(doi).await {
            Ok(url) => url,
            Err(e) => {
                tracing::warn!(doi = doi, error = %e, "Unpaywall lookup failed");
                None
            }
        };

        Ok(tool_ok(
            vec![ContentBlock::text(format!(
                "{} ({})",
                metadata.title,
                metadata.year.map(|y| y.to_string()).unwrap_or_default()
            ))],
            Some(json!({
                "doi": metadata.doi,
                "title": metadata.title,
                "authors": metadata.authors,
                "publisher": metadata.publisher,
                "year": metadata.year,
                "url": metadata.url,
                "journal": metadata.journal,
                "pdf_url": pdf_url,
            })),
        ))
    }

    pub(crate) async fn fetch_pdf_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
        let doi = args
            .get("doi")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing required parameter: doi"))?;

        let unpaywall = UnpaywallClient::default();
        let pdf_url = unpaywall
            .find_pdf_url(doi)
            .await?
            .ok_or_else(|| anyhow!("No open-access PDF found for DOI: {doi}"))?;

        let pdf_url = reqwest::Url::parse(&pdf_url)
            .map_err(|e| anyhow!("Unpaywall returned an invalid PDF URL {pdf_url}: {e}"))?;
        check_pdf_url(&pdf_url)?;

        let cache = ResearchCache::open()?;
        let pdf_path = cache.pdf_dir().join(pdf_cache_file_name(doi));

        // A cached file counts only if it is a PDF; anything else (an error
        // page or a file written by an older, non-atomic version) is fetched
        // again.
        let cached = has_pdf_magic(&pdf_path);
        if !cached {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    if attempt.previous().len() >= MAX_PDF_REDIRECTS {
                        attempt.error("too many redirects")
                    } else if let Err(e) = check_pdf_url(attempt.url()) {
                        attempt.error(e.to_string())
                    } else {
                        attempt.follow()
                    }
                }))
                .build()?;
            download_pdf(&client, pdf_url.clone(), &pdf_path, MAX_PDF_BYTES).await?;
        }

        let path_str = pdf_path.to_string_lossy().to_string();

        Ok(tool_ok(
            vec![ContentBlock::text(format!("PDF cached at: {path_str}"))],
            Some(json!({
                "path": path_str,
                "doi": doi,
                "url": pdf_url.as_str(),
                "cached": cached,
            })),
        ))
    }

    // --- #169: Advanced Research Tools ---

    pub(crate) fn query_knowledge_graph_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
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

            Ok(tool_ok(
                vec![ContentBlock::text(format!(
                    "Node {}: {} outgoing, {} incoming edges",
                    node_id,
                    edges_from.len(),
                    edges_to.len()
                ))],
                Some(json!({
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
            ))
        } else if let Some(query) = args.get("query").and_then(|v| v.as_str()) {
            let kind = match args.get("kind").and_then(|v| v.as_str()) {
                Some(s) => {
                    let k: NodeKind = serde_json::from_value(json!(s))
                        .map_err(|_| anyhow!("Unknown node kind: {s}"))?;
                    Some(k)
                }
                None => None,
            };

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

            Ok(tool_ok(
                vec![ContentBlock::text(format!("Found {} nodes", nodes.len()))],
                Some(json!({ "nodes": node_json, "count": nodes.len() })),
            ))
        } else {
            let (node_count, edge_count) = kg.stats()?;
            Ok(tool_ok(
                vec![ContentBlock::text(format!(
                    "Knowledge graph: {} nodes, {} edges",
                    node_count, edge_count
                ))],
                Some(json!({
                    "node_count": node_count,
                    "edge_count": edge_count,
                })),
            ))
        }
    }

    pub(crate) fn add_knowledge_node_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
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

        let kind: NodeKind = serde_json::from_value(json!(kind_str))
            .map_err(|_| anyhow!("Unknown node kind: {kind_str}"))?;

        let db_path = tome_cache_dir()?.join("knowledge.db");
        let kg = KnowledgeGraph::open(&db_path)?;
        kg.add_node(id, kind, label, metadata.as_deref())?;

        Ok(tool_ok(
            vec![ContentBlock::text(format!(
                "Added node '{id}' ({kind_str}): {label}"
            ))],
            Some(json!({"id": id, "kind": kind_str, "label": label})),
        ))
    }

    pub(crate) fn link_knowledge_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
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

        let kind: EdgeKind = serde_json::from_value(json!(kind_str))
            .map_err(|_| anyhow!("Unknown edge kind: {kind_str}"))?;

        let db_path = tome_cache_dir()?.join("knowledge.db");
        let kg = KnowledgeGraph::open(&db_path)?;
        kg.add_edge(source_id, target_id, kind, weight, metadata.as_deref())?;

        Ok(tool_ok(
            vec![ContentBlock::text(format!(
                "Linked {source_id} --{kind_str}--> {target_id}"
            ))],
            Some(json!({
                "source_id": source_id,
                "target_id": target_id,
                "kind": kind_str,
                "weight": weight,
            })),
        ))
    }

    pub(crate) fn track_citations_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
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

                Ok(tool_ok(
                    vec![ContentBlock::text(format!("Now tracking: {title}"))],
                    Some(json!({"paper_id": paper_id, "title": title, "action": "tracked"})),
                ))
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

                Ok(tool_ok(
                    vec![ContentBlock::text(format!(
                        "{} forward citations",
                        citations.len()
                    ))],
                    Some(json!({
                        "citations": citation_json,
                        "count": citations.len(),
                        "direction": "forward",
                    })),
                ))
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

                Ok(tool_ok(
                    vec![ContentBlock::text(format!(
                        "{} backward citations",
                        citations.len()
                    ))],
                    Some(json!({
                        "citations": citation_json,
                        "count": citations.len(),
                        "direction": "backward",
                    })),
                ))
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

        Ok(tool_ok(
            vec![ContentBlock::text(format!(
                "Improving {} vs degrading {}: {} applicable principles",
                improve_str,
                degrades_str,
                principles.len()
            ))],
            Some(json!({
                "improve": improve_str,
                "degrades": degrades_str,
                "principles": principle_json,
                "count": principles.len(),
            })),
        ))
    }
}

fn parse_parameter(s: &str) -> Result<Parameter> {
    serde_json::from_value(serde_json::Value::String(s.to_owned()))
        .map_err(|_| anyhow!("Unknown parameter: {s}"))
}
