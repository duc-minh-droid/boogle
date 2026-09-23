// Boogle web UI. Talks to `boogle serve` (/api/*). When no server is
// reachable (e.g. the page is opened from disk or a static host), it falls
// back to demo.json, which is produced by `boogle export` from the same engine.
"use strict";

const $ = (s, el = document) => el.querySelector(s);
const COLORS = ["#4285f4", "#ea4335", "#f9ab00", "#34a853", "#a142f4", "#12b5cb"];
const SUGGEST = ["rust programming", "running search engines", "big cats", "moon landing astronauts",
  "vietnamese noodle soup", "quantum cryptography", "ancient pyramids egypt", "jazz piano"];

let offline = null;      // demo.json contents when running without a server
let current = null;      // { q, search, explain }
let runId = 0;           // cancels stale explain animations

// ---------------------------------------------------------------- data ----
const api = {
  async stats() {
    try {
      const r = await fetch("api/stats");
      if (!r.ok) throw 0;
      return await r.json();
    } catch {
      const r = await fetch("demo.json");
      offline = await r.json();
      return { stats: offline.stats, build_ms: offline.build_ms };
    }
  },
  entry(q) {
    const key = q.trim().toLowerCase().replace(/\s+/g, " ");
    return offline.queries.find(e => e.query.toLowerCase() === key);
  },
  async search(q) {
    if (offline) return api.entry(q)?.search ?? null;
    return (await fetch(`api/search?n=10&q=${encodeURIComponent(q)}`)).json();
  },
  async explain(q) {
    if (offline) return api.entry(q)?.explain ?? null;
    return (await fetch(`api/explain?n=10&q=${encodeURIComponent(q)}`)).json();
  },
  async doc(id) {
    if (offline) return { id, ...offline.docs[id] };
    return (await fetch(`api/doc?id=${id}`)).json();
  },
};

// ---------------------------------------------------------------- utils ---
const esc = s => s.replace(/[&<>"]/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
const segs = list => list.map(s => s.hit ? `<mark>${esc(s.text)}</mark>` : esc(s.text)).join("");
const fmt = (x, d = 3) => Number(x).toFixed(d);
const sleep = ms => new Promise(r => setTimeout(r, ms));
const colorOf = (terms, t) => COLORS[terms.indexOf(t) % COLORS.length];

// ---------------------------------------------------------------- views ---
function showHome() {
  document.body.className = "home";
  $("#home").hidden = false;
  $("#topbar").hidden = $("#results-view").hidden = true;
  $("#home-q").value = "";
  $("#home-q").focus();
}

function showResultsChrome() {
  document.body.classList.remove("home");
  $("#home").hidden = true;
  $("#topbar").hidden = $("#results-view").hidden = false;
}

function setExplain(on) {
  document.body.classList.toggle("explaining", on);
  $("#btn-explain").setAttribute("aria-pressed", on);
  if (on && current) renderExplain(current.explain);
}

async function run(q, { explain = document.body.classList.contains("explaining"), push = true, focus = null } = {}) {
  q = q.trim();
  if (!q) return showHome();
  if (push) history.pushState({}, "", `?q=${encodeURIComponent(q)}${explain ? "&explain=1" : ""}`);
  showResultsChrome();
  $("#top-q").value = q;
  document.title = `${q} - Boogle`;
  const [search, ex] = await Promise.all([api.search(q), api.explain(q)]);
  current = { q, search, explain: ex };
  renderResults(search, q);
  document.body.classList.toggle("explaining", explain && !!ex);
  $("#btn-explain").setAttribute("aria-pressed", explain && !!ex);
  if (explain && ex) renderExplain(ex, focus);
}

function renderResults(r, q) {
  const hits = $("#hits");
  hits.innerHTML = "";
  if (!r) {
    $("#meta").innerHTML = "";
    hits.innerHTML = `<li class="empty">This is the offline demo, so only a few queries are pre-computed.
      Run <b>cargo run --release -- serve</b> for live search, or try:<br>${SUGGEST.map(s => `<a class="sugg" href="?q=${encodeURIComponent(s)}">${esc(s)}</a>`).join(" &middot; ")}</li>`;
    return;
  }
  const terms = r.terms.map(t => `<code style="color:${colorOf(r.terms, t)}">${esc(t)}</code>`).join(" ");
  $("#meta").innerHTML = `About <b>${r.total}</b> result${r.total === 1 ? "" : "s"} (<b>${fmt(r.took_ms, 2)}</b> ms) &nbsp;&middot;&nbsp; index terms: ${terms || "none"}`;
  if (!r.results.length) {
    hits.innerHTML = `<li class="empty">Your search - <b>${esc(q)}</b> - did not match any documents.<br>
      Every word is stemmed first, so try a different word rather than a different form of the same one.</li>`;
    return;
  }
  const top = r.results[0].score || 1;
  r.results.forEach((h, i) => {
    const li = document.createElement("li");
    li.className = "hit";
    li.dataset.id = h.id;
    li.style.animationDelay = `${i * 45}ms`;
    const title = h.title.map(s => s.text).join("");
    li.innerHTML = `
      <div class="crumb"><span class="fav">${esc(title[0] || "?")}</span>
        <span><span class="site">boogle.wiki</span><br>doc #${h.id}</span></div>
      <h3><a data-doc="${h.id}">${segs(h.title)}</a></h3>
      <p>${segs(h.snippet)}</p>
      <div class="row"><span class="scorebar"><span style="background:${COLORS[0]}"></span></span>
        score ${fmt(h.score, 4)} <button class="why" data-why="${h.id}">why this rank?</button></div>`;
    hits.appendChild(li);
    requestAnimationFrame(() => requestAnimationFrame(() => {
      $(".scorebar span", li).style.width = `${(h.score / top) * 100}%`;
    }));
  });
}

// --------------------------------------------------------- explain panel --
async function renderExplain(ex, focusId = null) {
  const my = ++runId;
  const alive = () => my === runId;
  const body = $("#ex-body");
  const terms = ex.terms.map(t => t.term);
  const N = ex.num_docs;
  body.innerHTML = "";
  const sec = (n, title, html) => {
    const d = document.createElement("div");
    d.className = "ex-sec";
    d.innerHTML = `<h4><span class="n">${n}</span>${title}</h4>${html}`;
    body.appendChild(d);
    requestAnimationFrame(() => d.classList.add("on"));
    return d;
  };

  // 1. tokenize
  const s1 = sec(1, "Tokenize &amp; stem the query", `<div class="tok-head"><span>word</span><span></span><span>clean</span><span></span><span>stem</span></div>`);
  const rows = ex.steps.map(s => {
    const row = document.createElement("div");
    row.className = "tok" + (s.kept ? "" : " dropped");
    const col = s.kept ? colorOf(terms, s.stem) : "";
    row.innerHTML = `<span class="c">${esc(s.raw)}</span><span class="arr">&rarr;</span><span class="c">${esc(s.cleaned || "∅")}</span>
      <span class="arr">&rarr;</span><span class="c stem" style="background:${col}">${s.kept ? esc(s.stem) : "dropped"}</span>`;
    s1.appendChild(row);
    return row;
  });
  if (!ex.steps.length) s1.insertAdjacentHTML("beforeend", `<p class="note">Empty query.</p>`);
  for (const row of rows) {
    for (const el of row.children) {
      if (!alive()) return;
      el.classList.add("on");
      await sleep(70);
    }
  }
  s1.insertAdjacentHTML("beforeend", `<p class="note">Lowercase, keep only letters and digits, then the Snowball English stemmer. The index was built with the same function, so <i>running</i> and <i>runs</i> both become <code>run</code>.</p>`);
  await sleep(250);

  // 2. posting lists
  const buckets = Math.min(N, 410);
  const cell = id => Math.floor((id * buckets) / N);
  const cols = Math.ceil(buckets / 5);
  const strip = () => `<div class="strip" style="--cols:${cols}">${"<i></i>".repeat(buckets)}</div>`;
  const s2 = sec(2, `Look up posting lists <span style="text-transform:none;letter-spacing:0">(${N} docs, one cell each)</span>`, "");
  const lists = ex.terms.map(t => {
    const d = document.createElement("div");
    d.className = "plist";
    d.innerHTML = `<div class="lbl"><code style="color:${colorOf(terms, t.term)}">${esc(t.term)}</code><span>df = ${t.df}</span></div>${strip()}`;
    s2.appendChild(d);
    return d;
  });
  for (let k = 0; k < ex.terms.length; k++) {
    const t = ex.terms[k], cells = lists[k].querySelectorAll("i"), col = colorOf(terms, t.term);
    const step = Math.max(1, Math.ceil(t.postings.length / 18));
    for (let j = 0; j < t.postings.length; j++) {
      const c = cells[cell(t.postings[j])];
      c.style.background = col;
      c.classList.add("on");
      if (j % step === 0) { await sleep(16); if (!alive()) return; }
    }
    if (!t.postings.length) lists[k].insertAdjacentHTML("beforeend", `<p class="note">Not in the index: this term adds nothing.</p>`);
    await sleep(120);
  }
  const u = document.createElement("div");
  u.className = "plist";
  u.innerHTML = `<div class="lbl"><code>union</code><span>${ex.candidates} candidate doc${ex.candidates === 1 ? "" : "s"}</span></div>${strip()}`;
  s2.appendChild(u);
  const ucells = u.querySelectorAll("i");
  const all = new Set(ex.terms.flatMap(t => t.postings));
  for (const id of all) { ucells[cell(id)].style.background = "#202124"; }
  s2.insertAdjacentHTML("beforeend", `<p class="note">Only these documents get scored. Everything else is skipped without being read.</p>`);
  await sleep(300);
  if (!alive()) return;

  // 3. idf
  const maxIdf = Math.log(N) || 1;
  const s3 = sec(3, "Inverse document frequency", "");
  const idfBars = ex.terms.map(t => {
    const d = document.createElement("div");
    d.className = "idf";
    d.innerHTML = `<code style="color:${colorOf(terms, t.term)}">${esc(t.term)}</code><div class="track"><span style="background:${colorOf(terms, t.term)}"></span></div>
      <span class="f">ln(${N}/${t.df || "-"}) = ${fmt(t.idf)}</span>`;
    s3.appendChild(d);
    return [d, t];
  });
  await sleep(50);
  for (const [d, t] of idfBars) { $(".track span", d).style.width = `${(t.idf / maxIdf) * 100}%`; await sleep(120); }
  s3.insertAdjacentHTML("beforeend", `<p class="note">Rare terms weigh more. A term in every document would score ln(1) = 0.</p>`);
  await sleep(400);
  if (!alive()) return;

  // 4. score breakdown
  const s4 = sec(4, "Score = &Sigma; tf &times; idf", `
    <div class="formula">score(d) = &Sigma;<sub>t</sub> tf(t,d) &times; idf(t)<br>tf(t,d) = count(t,d) / length(d)</div>
    <div class="legend" style="margin-top:10px">${terms.map(t => `<span><i style="background:${colorOf(terms, t)}"></i>${esc(t)}</span>`).join("")}</div>`);
  const maxScore = ex.docs[0]?.score || 1;
  const docRows = ex.docs.map(d => {
    const el = document.createElement("div");
    el.className = "sdoc";
    el.dataset.id = d.id;
    el.innerHTML = `<div class="lbl"><b>${esc(d.title)}</b><span>${fmt(d.score, 4)}</span></div>
      <div class="stack">${d.parts.map(p => `<span title="${esc(p.term)}: ${fmt(p.contrib, 4)}" style="background:${colorOf(terms, p.term)}"></span>`).join("")}</div>
      <div class="detail">length(d) = ${d.length} tokens<br>${d.parts.map(p =>
        `<i style="background:${colorOf(terms, p.term)}"></i>${esc(p.term).padEnd(8, " ")} ${p.count}/${d.length} &times; ${fmt(p.idf)} = ${fmt(p.contrib, 4)}`).join("<br>")}
        <br><b>total = ${fmt(d.score, 4)}</b></div>`;
    el.addEventListener("click", () => focusDoc(d.id, true));
    s4.appendChild(el);
    return [el, d];
  });
  if (!ex.docs.length) s4.insertAdjacentHTML("beforeend", `<p class="note">No candidates, nothing to score.</p>`);
  await sleep(60);
  for (const [el, d] of docRows) {
    if (!alive()) return;
    el.querySelectorAll(".stack span").forEach((s, i) => { s.style.width = `${(d.parts[i].contrib / maxScore) * 100}%`; });
    await sleep(90);
  }
  const fid = focusId ?? ex.docs[0]?.id;
  if (fid != null) focusDoc(fid, focusId != null);
}

function focusDoc(id, scroll) {
  document.querySelectorAll(".sdoc, .hit").forEach(e => e.classList.toggle("focus", e.dataset.id == id));
  const el = document.querySelector(`.sdoc[data-id="${id}"]`);
  if (el && scroll) el.scrollIntoView({ behavior: "smooth", block: "nearest" });
}

// ------------------------------------------------------------- doc modal --
async function openDoc(id) {
  const d = await api.doc(id);
  const terms = current?.search?.terms ?? [];
  // Client-side approximation of the stem match: a word is marked when its
  // cleaned form starts with a query stem ("engines" starts with "engin").
  const html = esc(d.content).split(" ").map(w => {
    const c = w.toLowerCase().replace(/[^\p{L}\p{N}]/gu, "");
    return c && terms.some(t => c.startsWith(t)) ? `<mark>${w}</mark>` : w;
  }).join(" ");
  $("#doc-title").textContent = d.title;
  $("#doc-content").innerHTML = html;
  $("#doc-modal").hidden = false;
}

// ------------------------------------------------------------ googly eyes --
function eyes() {
  const pupils = [...document.querySelectorAll(".eye i")];
  document.addEventListener("mousemove", e => {
    for (const p of pupils) {
      const r = p.parentElement.getBoundingClientRect();
      const dx = e.clientX - (r.left + r.width / 2), dy = e.clientY - (r.top + r.height / 2);
      const a = Math.atan2(dy, dx), m = Math.min(r.width * 0.16, Math.hypot(dx, dy) / 12);
      p.style.transform = `translate(${Math.cos(a) * m}px, ${Math.sin(a) * m}px)`;
    }
  });
  setInterval(() => {
    document.querySelectorAll(".eye").forEach(e => { e.classList.remove("blink"); void e.offsetWidth; e.classList.add("blink"); });
  }, 4200);
}

// ----------------------------------------------------------------- boot ---
async function boot() {
  eyes();
  $("#suggestions").innerHTML = SUGGEST.map(s => `<a class="sugg">${esc(s)}</a>`).join("");
  document.addEventListener("click", e => {
    const a = e.target.closest("a.sugg");
    if (a) { e.preventDefault(); run(a.textContent); }
    const doc = e.target.closest("[data-doc]");
    if (doc) openDoc(+doc.dataset.doc);
    const why = e.target.closest("[data-why]");
    if (why) {
      const id = +why.dataset.why;
      if (!document.body.classList.contains("explaining")) {
        document.body.classList.add("explaining");
        $("#btn-explain").setAttribute("aria-pressed", true);
        renderExplain(current.explain, id);
      } else focusDoc(id, true);
    }
  });
  $("#home-form").addEventListener("submit", e => { e.preventDefault(); run($("#home-q").value); });
  $("#top-form").addEventListener("submit", e => { e.preventDefault(); run($("#top-q").value); });
  $("#btn-search").addEventListener("click", () => run($("#home-q").value));
  $("#btn-lucky").addEventListener("click", async () => {
    const q = $("#home-q").value.trim() || SUGGEST[Math.floor(Math.random() * SUGGEST.length)];
    await run(q);
    const first = current?.search?.results?.[0];
    if (first) openDoc(first.id);
  });
  $("#btn-explain").addEventListener("click", () => setExplain(!document.body.classList.contains("explaining")));
  $("#ex-close").addEventListener("click", () => setExplain(false));
  $("#home-link").addEventListener("click", e => { e.preventDefault(); history.pushState({}, "", "./"); showHome(); });
  $("#doc-close").addEventListener("click", () => ($("#doc-modal").hidden = true));
  $("#doc-modal").addEventListener("click", e => { if (e.target.id === "doc-modal") e.target.hidden = true; });
  document.addEventListener("keydown", e => {
    if (e.key === "Escape") $("#doc-modal").hidden = true;
    if (e.key === "/" && document.activeElement.tagName !== "INPUT") { e.preventDefault(); ($("#home").hidden ? $("#top-q") : $("#home-q")).focus(); }
  });
  window.addEventListener("popstate", fromUrl);

  const s = await api.stats();
  const st = s.stats;
  $("#stats").textContent = `${st.num_docs.toLocaleString()} documents · ${st.num_terms.toLocaleString()} terms · ${st.num_tokens.toLocaleString()} tokens · indexed in ${fmt(s.build_ms, 1)} ms`;
  $("#home-q").placeholder = `Search ${st.num_docs} articles`;
  $("#mode").textContent = offline ? "offline demo (pre-computed by boogle export)" : "live: boogle serve";
  fromUrl();
}

function fromUrl() {
  const p = new URLSearchParams(location.search);
  const q = p.get("q");
  if (q) run(q, { explain: p.get("explain") === "1", push: false });
  else showHome();
}

boot();
