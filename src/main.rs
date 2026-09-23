mod engine;
mod server;

use engine::{Document, Index};
use std::io::{self, BufRead, Write};
use std::time::Instant;

const USAGE: &str = "\
boogle - a tiny TF-IDF search engine

USAGE:
    boogle [OPTIONS]                      interactive prompt (default)
    boogle search <query...> [--json]     print the top results once
    boogle explain <query...> [--json]    show tokens, postings and score breakdown
    boogle serve [--port 8107]            JSON API + web UI on http://localhost:<port>
    boogle export <file.json> <query>...  write search+explain JSON for the offline web demo

OPTIONS:
    --corpus <path>   corpus file (default: wiki.txt)
    --limit <n>       only index the first n documents
    -n <n>            number of results to show (default 5)
    --web <dir>       static files for `serve` (default: web)
";

struct Opts {
    cmd: String,
    args: Vec<String>,
    corpus: String,
    limit: Option<usize>,
    n: usize,
    json: bool,
    port: u16,
    web: String,
}

fn parse_args() -> Result<Opts, String> {
    let mut it = std::env::args().skip(1);
    let mut o = Opts {
        cmd: "repl".into(),
        args: vec![],
        corpus: "wiki.txt".into(),
        limit: None,
        n: 5,
        json: false,
        port: 8107,
        web: "web".into(),
    };
    let mut first = true;
    while let Some(a) = it.next() {
        let mut val = |name: &str| it.next().ok_or(format!("{name} needs a value"));
        match a.as_str() {
            "-h" | "--help" => return Err(String::new()),
            "--corpus" => o.corpus = val("--corpus")?,
            "--limit" => o.limit = Some(val("--limit")?.parse().map_err(|_| "--limit: not a number")?),
            "-n" => o.n = val("-n")?.parse().map_err(|_| "-n: not a number")?,
            "--port" => o.port = val("--port")?.parse().map_err(|_| "--port: not a number")?,
            "--web" => o.web = val("--web")?,
            "--json" => o.json = true,
            _ if first && matches!(a.as_str(), "repl" | "search" | "explain" | "serve" | "export") => o.cmd = a,
            _ => o.args.push(a),
        }
        first = false;
    }
    Ok(o)
}

fn load(o: &Opts) -> (Vec<Document>, Index, f64) {
    let mut documents = match engine::parse_documents(&o.corpus) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", o.corpus);
            std::process::exit(1);
        }
    };
    if let Some(n) = o.limit {
        documents.truncate(n);
    }
    let t = Instant::now();
    let index = Index::build(&documents);
    (documents, index, t.elapsed().as_secs_f64() * 1000.0)
}

fn print_results(index: &Index, documents: &[Document], query: &str, n: usize) {
    let results = index.search(query);
    if results.is_empty() {
        println!("  (no results)");
    }
    for (doc_id, score) in results.iter().take(n) {
        println!("{score:.3} -> {}", documents[*doc_id].title);
    }
}

fn print_explain(e: &engine::ExplainResponse) {
    println!("tokenize:");
    for s in &e.steps {
        if s.kept {
            println!("  {:<16} -> {:<14} -> {}", s.raw, s.cleaned, s.stem);
        } else {
            println!("  {:<16} -> (dropped)", s.raw);
        }
    }
    println!("\nterms (N = {} docs):", e.num_docs);
    for t in &e.terms {
        println!("  {:<12} df={:<4} idf=ln({}/{})={:.3}", t.term, t.df, e.num_docs, t.df.max(1), t.idf);
    }
    println!("\n{} candidate docs; top {}:", e.candidates, e.docs.len());
    for d in &e.docs {
        println!("  {:.4}  {}  (len {})", d.score, d.title, d.length);
        for p in d.parts.iter().filter(|p| p.count > 0) {
            println!(
                "          {:<12} tf={}/{}={:.4} x idf {:.3} = {:.4}",
                p.term, p.count, d.length, p.tf, p.idf, p.contrib
            );
        }
    }
}

fn main() {
    let o = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("error: {e}\n");
            }
            eprint!("{USAGE}");
            std::process::exit(if e.is_empty() { 0 } else { 2 });
        }
    };

    let (documents, index, build_ms) = load(&o);
    let query = o.args.join(" ");

    match o.cmd.as_str() {
        "search" => {
            if o.json {
                let r = engine::search_response(&index, &documents, &query, o.n);
                println!("{}", serde_json::to_string_pretty(&r).unwrap());
            } else {
                print_results(&index, &documents, &query, o.n);
            }
        }
        "explain" => {
            let e = engine::explain_response(&index, &documents, &query, o.n);
            if o.json {
                println!("{}", serde_json::to_string_pretty(&e).unwrap());
            } else {
                print_explain(&e);
            }
        }
        "serve" => server::run(&documents, &index, build_ms, o.port, &o.web),
        "export" => {
            let Some((path, queries)) = o.args.split_first() else {
                eprintln!("usage: boogle export <file.json> <query>...");
                std::process::exit(2);
            };
            let entries: Vec<serde_json::Value> = queries
                .iter()
                .map(|q| {
                    serde_json::json!({
                        "query": q,
                        "search": engine::search_response(&index, &documents, q, 10),
                        "explain": engine::explain_response(&index, &documents, q, 10),
                    })
                })
                .collect();
            let docs: Vec<serde_json::Value> = documents
                .iter()
                .map(|d| serde_json::json!({"title": d.title, "content": d.content}))
                .collect();
            let out = serde_json::json!({
                "stats": index.stats(), "build_ms": build_ms, "queries": entries, "docs": docs
            });
            std::fs::write(path, serde_json::to_string(&out).unwrap()).expect("write export");
            eprintln!("wrote {} queries to {path}", queries.len());
        }
        _ => {
            let s = index.stats();
            println!("indexed {} docs, {} terms in {build_ms:.1} ms  (:explain <q> for details, :q to quit)", s.num_docs, s.num_terms);
            let stdin = io::stdin();
            loop {
                print!("search> ");
                io::stdout().flush().unwrap();
                let mut line = String::new();
                // EOF (Ctrl+D / Ctrl+Z) ends the session instead of spinning forever.
                if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
                    println!();
                    break;
                }
                let q = line.trim();
                match q {
                    "" => continue,
                    ":q" | ":quit" | "exit" => break,
                    _ if q.starts_with(":explain ") => {
                        print_explain(&engine::explain_response(&index, &documents, &q[9..], o.n))
                    }
                    _ => print_results(&index, &documents, q, o.n),
                }
            }
        }
    }
}
