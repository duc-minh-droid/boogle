//! Parsing, tokenizing, indexing and TF-IDF ranking.
//!
//! Everything the CLI and the HTTP server need lives here, so both front ends
//! run exactly the same code path.

use rust_stemmers::{Algorithm, Stemmer};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::sync::OnceLock;

fn stemmer() -> &'static Stemmer {
    // Creating a stemmer is cheap-ish, but tokenize() runs once per document,
    // so build it once and share it.
    static STEMMER: OnceLock<Stemmer> = OnceLock::new();
    STEMMER.get_or_init(|| Stemmer::create(Algorithm::English))
}

/// One word going through the tokenizer, kept for the explain view.
#[derive(Serialize, Clone)]
pub struct TokenStep {
    pub raw: String,
    pub lower: String,
    pub cleaned: String,
    pub stem: String,
    /// false when the word was pure punctuation and got dropped
    pub kept: bool,
}

fn normalize(word: &str) -> (String, String) {
    let lower = word.to_lowercase();
    let cleaned: String = lower.chars().filter(|c| c.is_alphanumeric()).collect();
    (lower, cleaned)
}

/// Lowercase, strip punctuation, split on whitespace, stem.
pub fn tokenize(input: &str) -> Vec<String> {
    input
        .split_whitespace()
        .filter_map(|w| {
            let (_, cleaned) = normalize(w);
            // A word like "-" or "&" cleans down to nothing; indexing "" as a
            // term would make every such document match an empty query.
            (!cleaned.is_empty()).then(|| stemmer().stem(&cleaned).into_owned())
        })
        .collect()
}

/// Same as `tokenize`, but records every intermediate stage.
pub fn tokenize_steps(input: &str) -> Vec<TokenStep> {
    input
        .split_whitespace()
        .map(|raw| {
            let (lower, cleaned) = normalize(raw);
            let kept = !cleaned.is_empty();
            let stem = if kept { stemmer().stem(&cleaned).into_owned() } else { String::new() };
            TokenStep { raw: raw.to_string(), lower, cleaned, stem, kept }
        })
        .collect()
}

pub struct Document {
    pub id: usize,
    pub title: String,
    pub content: String,
}

impl Document {
    fn text(&self) -> String {
        format!("{} {}", self.title, self.content)
    }
}

/// Articles are separated by two or more blank lines. The first line of each
/// block is the title, the rest is content.
pub fn parse_documents(path: &str) -> std::io::Result<Vec<Document>> {
    let text = fs::read_to_string(path)?;
    Ok(parse_text(&text))
}

pub fn parse_text(text: &str) -> Vec<Document> {
    let mut blocks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut blank_run = 0;

    for line in text.lines() {
        if line.trim().is_empty() {
            blank_run += 1;
        } else {
            if blank_run >= 2 && !current.trim().is_empty() {
                blocks.push(std::mem::take(&mut current));
            }
            blank_run = 0;
            current.push_str(line);
            current.push('\n');
        }
    }
    if !current.trim().is_empty() {
        blocks.push(current);
    }

    let mut documents = Vec::new();
    for block in blocks {
        let mut lines = block.lines();
        let title = match lines.next() {
            Some(t) if !t.trim().is_empty() => t.trim().to_string(),
            _ => continue,
        };
        let content: String = lines
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" ");

        documents.push(Document { id: documents.len(), title, content });
    }

    documents
}

pub struct Index {
    posting_list: HashMap<String, BTreeSet<usize>>,
    freq: HashMap<(String, usize), i32>,
    doc_lengths: Vec<usize>,
    num_docs: usize,
}

#[derive(Serialize)]
pub struct Stats {
    pub num_docs: usize,
    pub num_terms: usize,
    pub num_tokens: usize,
    pub num_postings: usize,
}

impl Index {
    pub fn build(documents: &[Document]) -> Self {
        let mut posting_list: HashMap<String, BTreeSet<usize>> = HashMap::new();
        let mut freq: HashMap<(String, usize), i32> = HashMap::new();
        let mut doc_lengths = vec![0usize; documents.len()];

        for doc in documents {
            let tokens = tokenize(&doc.text());
            doc_lengths[doc.id] = tokens.len();
            for token in tokens {
                posting_list.entry(token.clone()).or_default().insert(doc.id);
                *freq.entry((token, doc.id)).or_insert(0) += 1;
            }
        }

        Index { posting_list, freq, doc_lengths, num_docs: documents.len() }
    }

    pub fn stats(&self) -> Stats {
        Stats {
            num_docs: self.num_docs,
            num_terms: self.posting_list.len(),
            num_tokens: self.doc_lengths.iter().sum(),
            num_postings: self.freq.len(),
        }
    }

    fn union(&self, terms: &[String]) -> Vec<usize> {
        let mut result = BTreeSet::new();
        for term in terms {
            if let Some(postings) = self.posting_list.get(term) {
                result.extend(postings);
            }
        }
        result.into_iter().collect()
    }

    pub fn idf(&self, term: &str) -> f64 {
        match self.posting_list.get(term) {
            Some(postings) => (self.num_docs as f64 / postings.len() as f64).ln(),
            None => 0.0, // term not in corpus at all
        }
    }

    fn count(&self, term: &str, d: usize) -> i32 {
        *self.freq.get(&(term.to_string(), d)).unwrap_or(&0)
    }

    fn tf(&self, term: &str, d: usize) -> f64 {
        let len = self.doc_lengths[d];
        if len == 0 { 0.0 } else { self.count(term, d) as f64 / len as f64 }
    }

    fn score(&self, terms: &[String], d: usize) -> f64 {
        terms.iter().map(|t| self.tf(t, d) * self.idf(t)).sum()
    }

    /// Returns (doc id, score) for every document containing at least one
    /// query term, best first.
    pub fn search(&self, query: &str) -> Vec<(usize, f64)> {
        let terms = query_terms(query);
        let candidates = self.union(&terms);

        let mut results: Vec<(usize, f64)> =
            candidates.into_iter().map(|idx| (idx, self.score(&terms, idx))).collect();

        // Ties (e.g. a term that is in every doc, idf = 0) fall back to doc order.
        results.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        results
    }
}

/// Tokenized query with duplicates removed, so "rust rust" does not count the
/// same term twice.
pub fn query_terms(query: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    tokenize(query).into_iter().filter(|t| seen.insert(t.clone())).collect()
}

// ---------------------------------------------------------------------------
// JSON-facing views: search results with snippets, and the explain trace.
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct Segment {
    pub text: String,
    pub hit: bool,
}

#[derive(Serialize)]
pub struct Hit {
    pub id: usize,
    pub title: Vec<Segment>,
    pub snippet: Vec<Segment>,
    pub score: f64,
}

#[derive(Serialize)]
pub struct SearchResponse {
    pub query: String,
    pub terms: Vec<String>,
    pub total: usize,
    pub took_ms: f64,
    pub results: Vec<Hit>,
}

#[derive(Serialize)]
pub struct TermInfo {
    pub term: String,
    pub df: usize,
    pub idf: f64,
    /// Doc ids of the posting list, sorted (BTreeSet), capped at 5000.
    pub postings: Vec<usize>,
}

#[derive(Serialize)]
pub struct TermPart {
    pub term: String,
    pub count: i32,
    pub tf: f64,
    pub idf: f64,
    pub contrib: f64,
}

#[derive(Serialize)]
pub struct DocExplain {
    pub id: usize,
    pub title: String,
    pub length: usize,
    pub score: f64,
    pub parts: Vec<TermPart>,
}

#[derive(Serialize)]
pub struct ExplainResponse {
    pub query: String,
    pub steps: Vec<TokenStep>,
    pub num_docs: usize,
    pub terms: Vec<TermInfo>,
    pub candidates: usize,
    pub docs: Vec<DocExplain>,
    pub took_ms: f64,
}

/// Split `text` into plain / highlighted runs. A word is a hit when its stem
/// is one of the query terms, which is the same test the index uses.
fn highlight(words: &[&str], terms: &HashSet<&str>) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    for (i, w) in words.iter().enumerate() {
        let (_, cleaned) = normalize(w);
        let hit = !cleaned.is_empty() && terms.contains(stemmer().stem(&cleaned).as_ref());
        let sep = if i == 0 { "" } else { " " };
        if hit {
            // keep punctuation outside the <mark>
            let start = w.find(|c: char| c.is_alphanumeric()).unwrap_or(0);
            let end = w.rfind(|c: char| c.is_alphanumeric()).map(|e| e + w[e..].chars().next().unwrap().len_utf8()).unwrap_or(w.len());
            push(&mut out, format!("{sep}{}", &w[..start]), false);
            push(&mut out, w[start..end].to_string(), true);
            push(&mut out, w[end..].to_string(), false);
        } else {
            push(&mut out, format!("{sep}{w}"), false);
        }
    }
    out
}

fn push(out: &mut Vec<Segment>, text: String, hit: bool) {
    if text.is_empty() {
        return;
    }
    match out.last_mut() {
        Some(last) if last.hit == hit && !hit => last.text.push_str(&text),
        _ => out.push(Segment { text, hit }),
    }
}

/// Pick the window of `width` words that contains the most query-term hits.
fn snippet(content: &str, terms: &HashSet<&str>, width: usize) -> Vec<Segment> {
    let words: Vec<&str> = content.split_whitespace().collect();
    if words.len() <= width {
        return highlight(&words, terms);
    }
    let is_hit: Vec<bool> = words
        .iter()
        .map(|w| {
            let (_, c) = normalize(w);
            !c.is_empty() && terms.contains(stemmer().stem(&c).as_ref())
        })
        .collect();

    let mut hits = is_hit[..width].iter().filter(|h| **h).count();
    let mut best = (hits, 0usize); // (hits in window, window start)
    for start in 1..=words.len() - width {
        if is_hit[start - 1] { hits -= 1; }
        if is_hit[start + width - 1] { hits += 1; }
        if hits > best.0 { best = (hits, start); }
    }
    // Back up a few words so the first hit is not glued to the left edge.
    let start = best.1.saturating_sub(4).min(words.len() - width);
    let end = start + width;

    let mut segs = Vec::new();
    if start > 0 { segs.push(Segment { text: "... ".into(), hit: false }); }
    segs.extend(highlight(&words[start..end], terms));
    if end < words.len() { push(&mut segs, " ...".into(), false); }
    segs
}

fn ms_since(t: std::time::Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

pub fn search_response(index: &Index, docs: &[Document], query: &str, limit: usize) -> SearchResponse {
    let t0 = std::time::Instant::now();
    let terms = query_terms(query);
    let ranked = index.search(query);
    let set: HashSet<&str> = terms.iter().map(|s| s.as_str()).collect();

    let results = ranked
        .iter()
        .take(limit)
        .map(|&(id, score)| {
            let d = &docs[id];
            let title_words: Vec<&str> = d.title.split_whitespace().collect();
            Hit { id, title: highlight(&title_words, &set), snippet: snippet(&d.content, &set, 32), score }
        })
        .collect();

    SearchResponse { query: query.to_string(), total: ranked.len(), terms, took_ms: ms_since(t0), results }
}

pub fn explain_response(index: &Index, docs: &[Document], query: &str, limit: usize) -> ExplainResponse {
    let t0 = std::time::Instant::now();
    let steps = tokenize_steps(query);
    let terms = query_terms(query);
    let ranked = index.search(query);

    let term_info = terms
        .iter()
        .map(|t| {
            let postings = index.posting_list.get(t);
            TermInfo {
                term: t.clone(),
                df: postings.map_or(0, |p| p.len()),
                idf: index.idf(t),
                postings: postings.map_or(vec![], |p| p.iter().copied().take(5000).collect()),
            }
        })
        .collect();

    let doc_ex = ranked
        .iter()
        .take(limit)
        .map(|&(id, score)| DocExplain {
            id,
            title: docs[id].title.clone(),
            length: index.doc_lengths[id],
            score,
            parts: terms
                .iter()
                .map(|t| {
                    let (tf, idf) = (index.tf(t, id), index.idf(t));
                    TermPart { term: t.clone(), count: index.count(t, id), tf, idf, contrib: tf * idf }
                })
                .collect(),
        })
        .collect();

    ExplainResponse {
        query: query.to_string(),
        steps,
        num_docs: index.num_docs,
        terms: term_info,
        candidates: ranked.len(),
        docs: doc_ex,
        took_ms: ms_since(t0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Rust\n\nRust is a systems programming language.\n\n\nPython\n\nPython is a scripting language. Snakes!\n\n\nCooking\n\nRunning a kitchen - boiling pasta.\n";

    #[test]
    fn parses_blocks() {
        let docs = parse_text(SAMPLE);
        assert_eq!(docs.len(), 3);
        assert_eq!(docs[1].title, "Python");
        assert!(docs[2].content.starts_with("Running"));
    }

    #[test]
    fn tokenizer_stems_and_drops_punctuation() {
        assert_eq!(tokenize("Running - Runs!"), vec!["run", "run"]);
        let steps = tokenize_steps("Running -");
        assert!(steps[0].kept && !steps[1].kept);
    }

    #[test]
    fn ranks_by_tfidf() {
        let docs = parse_text(SAMPLE);
        let index = Index::build(&docs);
        let r = index.search("python snakes");
        assert_eq!(r[0].0, 1);
        // "language" is in 2 of 3 docs, so the doc that only has "language" loses
        let r = index.search("rust language");
        assert_eq!(r[0].0, 0);
        assert!(index.search("").is_empty());
        assert!(index.search("zebra").is_empty());
    }

    #[test]
    fn snippet_highlights_stemmed_matches() {
        let docs = parse_text(SAMPLE);
        let index = Index::build(&docs);
        let resp = search_response(&index, &docs, "run", 5);
        let marked: Vec<&str> = resp.results[0].snippet.iter().filter(|s| s.hit).map(|s| s.text.as_str()).collect();
        assert_eq!(marked, vec!["Running"]);
    }
}
