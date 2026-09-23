//! `boogle serve`: a small blocking HTTP server (tiny_http) exposing the
//! engine as JSON and serving the web UI from a directory.

use crate::engine::{self, Document, Index};
use std::path::{Component, Path, PathBuf};
use tiny_http::{Header, Response, Server};

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(v) => {
                        out.push(v);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        (k == key).then(|| percent_decode(v))
    })
}

fn header(k: &str, v: &str) -> Header {
    Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap()
}

fn json(body: String) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(body)
        .with_header(header("Content-Type", "application/json; charset=utf-8"))
        .with_header(header("Access-Control-Allow-Origin", "*"))
}

fn mime(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}

/// Map a URL path onto the web dir, refusing anything that climbs out of it.
fn static_path(web: &str, url_path: &str) -> Option<PathBuf> {
    let rel = Path::new(url_path.trim_start_matches('/'));
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    let mut p = Path::new(web).join(rel);
    if url_path.ends_with('/') || url_path.is_empty() {
        p = p.join("index.html");
    }
    p.is_file().then_some(p)
}

pub fn run(documents: &[Document], index: &Index, build_ms: f64, port: u16, web: &str) {
    let addr = format!("127.0.0.1:{port}");
    let server = Server::http(&addr).unwrap_or_else(|e| {
        eprintln!("error: cannot bind {addr}: {e}");
        std::process::exit(1);
    });
    let s = index.stats();
    println!("indexed {} docs, {} terms in {build_ms:.1} ms", s.num_docs, s.num_terms);
    println!("boogle is listening on http://localhost:{port}  (web dir: {web})");

    for req in server.incoming_requests() {
        let url = req.url().to_string();
        let (path, qs) = url.split_once('?').unwrap_or((&url, ""));
        let n = param(qs, "n").and_then(|v| v.parse().ok()).unwrap_or(10usize).min(50);
        let q = param(qs, "q").unwrap_or_default();

        let resp = match path {
            "/api/search" => json(serde_json::to_string(&engine::search_response(index, documents, &q, n)).unwrap()),
            "/api/explain" => json(serde_json::to_string(&engine::explain_response(index, documents, &q, n)).unwrap()),
            "/api/stats" => {
                let v = serde_json::json!({"stats": index.stats(), "build_ms": build_ms});
                json(v.to_string())
            }
            "/api/doc" => match param(qs, "id").and_then(|v| v.parse::<usize>().ok()).and_then(|i| documents.get(i)) {
                Some(d) => json(serde_json::json!({"id": d.id, "title": d.title, "content": d.content}).to_string()),
                None => json(r#"{"error":"no such document"}"#.into()).with_status_code(404),
            },
            _ => match static_path(web, path) {
                Some(p) => match std::fs::read(&p) {
                    Ok(bytes) => Response::from_data(bytes).with_header(header("Content-Type", mime(&p))),
                    Err(_) => Response::from_string("read error").with_status_code(500),
                },
                None => Response::from_string("not found").with_status_code(404),
            },
        };
        let _ = req.respond(resp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_query_strings() {
        assert_eq!(percent_decode("rust+lang%20x%2B"), "rust lang x+");
        assert_eq!(percent_decode("caf%C3%A9"), "café");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(param("q=a+b&n=3", "n").as_deref(), Some("3"));
    }

    #[test]
    fn blocks_path_traversal() {
        assert!(static_path("web", "/../Cargo.toml").is_none());
    }
}
