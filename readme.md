# boogle

A tiny command-line search engine over a corpus of Wikipedia-style articles. It builds an inverted index in memory and ranks results with TF-IDF.

## How it works

1. **Parsing** — `parse_documents` reads `wiki.txt` and splits it into articles. Articles are separated by two or more blank lines; the first line of each block is the title, and everything after (until the next blank-line run) is the content.
2. **Tokenizing** — `tokenize` lowercases text, strips punctuation, splits on whitespace, and stems each word (English stemmer via `rust_stemmers`) so that e.g. "running" and "run" match the same term.
3. **Indexing** — `Index::build` walks the documents and builds:
    - `posting_list`: term → set of document ids containing it
    - `freq`: (term, doc id) → how many times that term appears in that doc
    - `doc_lengths`: doc id → total token count, used to normalize term frequency
4. **Searching** — `Index::search` tokenizes the query, finds the union of documents containing any query term, scores each with TF-IDF, and returns results sorted highest-score first.

## Requirements

- Rust (stable) and Cargo
- A `wiki.txt` file in the working directory, formatted as:

  ```
  Title One

  Paragraph content for the first article, can span
  multiple lines of the source file.


  Title Two

  Paragraph content for the second article.
  ```

  Articles are separated by a blank line, then a run of **two or more** blank lines before the next title.

## Setup

Add the stemming dependency to `Cargo.toml`:

```toml
[dependencies]
rust_stemmers = "1"
```

## Running

```bash
cargo run --release
```

`--release` matters here — indexing is noticeably slower in debug builds.

You'll get a prompt:

```
search> rust programming
0.842 -> Rust
0.531 -> Systems Programming
...
search> 
```

Type a query and press enter to see the top 5 matching titles with their scores. There's no exit command — use `Ctrl+C` to quit.

## Notes / current limitations

- **Only the first 2000 documents are indexed** (`Index::build(&documents[0..2000])` in `main`). This is a deliberate limit to keep indexing fast during development. To index the full corpus, change this to `Index::build(&documents)` — expect a slower startup.
- Input is assumed to always be valid UTF-8 text; there's no handling for empty queries, EOF (`Ctrl+D`), or malformed input.
- Scoring is plain TF-IDF (term frequency × inverse document frequency), summed across query terms — no phrase matching, ranking boosts, or fuzzy matching.