extern crate rust_stemmers;
use rust_stemmers::{Algorithm, Stemmer};
use std::collections::{HashMap, BTreeSet};
use std::fs;
use std::io::{self, Write};

fn tokenize(input: &str) -> Vec<String> {
    let en_stemmer = Stemmer::create(Algorithm::English);
    input.trim().to_lowercase().split_whitespace()
        .map(|token| {
            token.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
        })
        .map(|word| en_stemmer.stem(&word).into_owned())
        .collect()
}

struct Document {
    id: usize,
    title: String,
    content: String,
}

impl Document {
    fn text(&self) -> String {
        format!("{} {}", self.title, self.content)
    }
}

fn parse_documents(path: &str) -> Vec<Document> {
    let text = fs::read_to_string(path).expect("Failed to read file");

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

        documents.push(Document {
            id: documents.len(),
            title,
            content,
        });
    }

    documents
}

struct Index {
    posting_list: HashMap<String, BTreeSet<usize>>,
    freq: HashMap<(String, usize), i32>,
    doc_lengths: Vec<usize>,
    num_docs: usize,
}

impl Index {
    fn build(documents: &[Document]) -> Self {
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

        Index {
            posting_list,
            freq,
            doc_lengths,
            num_docs: documents.len(),
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

    fn score(&self, terms: &[String], d: usize) -> f64 {
        let mut total = 0.0;
        for t in terms {
            let count = *self.freq.get(&(t.clone(), d)).unwrap_or(&0) as f64;
            let tf = count / self.doc_lengths[d] as f64;
            let idf = match self.posting_list.get(t) {
                Some(postings) => (self.num_docs as f64 / postings.len() as f64).ln(),
                None => 0.0, // term not in corpus at all
            };
            total += tf * idf;
        }
        total
    }

    fn search(&self, query: &str, documents: &[Document]) -> Vec<(usize, f64)> {
        let terms = tokenize(query);
        let candidates = self.union(&terms);

        let mut results: Vec<(usize, f64)> = candidates
            .into_iter()
            .map(|idx| (idx, self.score(&terms, idx)))
            .collect();

        results.sort_by(|a, b| b.1.total_cmp(&a.1));
        let _ = documents; // kept in signature in case you want to slice titles etc.
        results
    }
}

fn main() {
    let documents = parse_documents("wiki.txt");
    let index = Index::build(&documents[0..2000]);
    loop {
        print!("search> ");
        io::stdout().flush().unwrap();
        let mut query = String::new();
        io::stdin().read_line(&mut query).unwrap();
        for (doc_id, score) in index.search(query.trim(), &documents).iter().take(5) {
            println!("{score:.3} -> {}", documents[*doc_id].title);
        }
    }
}
