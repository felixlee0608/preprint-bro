# preprint-bro

Paste an **arXiv, bioRxiv or medRxiv** paper and get an interactive course.
No login. One paper gets one course: whoever asks first starts the build, and
everyone after that is sent straight to it.

This is a fork of [aeroxy/paper-bro](https://github.com/aeroxy/paper-bro)
(MIT, see `LICENSE`), extended with bioRxiv/medRxiv support and multi-provider
LLM configuration, and packaged for **self-hosting**: each person runs their
own copy on their own machine, no cloud account, no shared queue. The original
project publishes courses to AERO; here the default (`PUBLISH=0`) keeps every
course in your own Postgres and serves it at `/preprint-bro/preview/<alias>`.

## What runs where

| Piece | Choice |
|---|---|
| Server | Rust: axum, tokio. One binary serving the SPA, the API, SSE progress, and the build workers |
| Database | Postgres via sqlx (raw SQL). Migrations in `migrations/` run at boot. Local or hosted, both work |
| Front end | React 19 + Vite + Tailwind 3, built with bun, embedded with rust-embed |
| Model | **Any OpenAI-compatible server**: hosted APIs, vLLM, Ollama, LM Studio… several can be configured side by side; pick per build |
| Papers | arXiv (API + HTML/LaTeX), bioRxiv and medRxiv (api.biorxiv.org + JATS XML) |
| Browser QA | `chrome-headless-shell` (Chrome for Testing, pinned), driven over CDP with chromiumoxide |
| Publishing | Optional. `PUBLISH=1` sends courses to AERO's `/mcp` endpoint; the default is local preview only |
| Deploy | One binary and a headless Chromium behind a reverse proxy at `/preprint-bro` |

A build goes: ingest → outline → lessons (6 in parallel) → browser QA →
review → repair (at most two rounds) → publish. Each stage checkpoints to
`course_artifacts`, so a restarted job resumes instead of paying again.
Progress streams to the paper page over SSE.

### Safety model, in one paragraph

Courses are served from the host's own origin, and they contain model-written
code influenced by untrusted paper text. So the browser confines that code:

- Lesson prose goes through ammonia.
- Every lab runs in a sandboxed iframe with an opaque origin (`sandbox="allow-scripts"`).
- Quiz and glossary text is rendered as text.
- A CSP `<meta>` limits scripts, images and network calls.

`preprint-bro qa-sample` shows a lab failing to read storage, cookies or the
network from inside its iframe.

## What you pay

- **A model.** Roughly **US$1 per course** with a frontier hosted API
  (6 lessons + outline + review + repairs). With a local model (Ollama etc.)
  it's free, but **small local models write noticeably worse courses** than
  the big hosted APIs — fine for experiments, not for polished results.
- **A database.** Your own Postgres; the free tier of a hosted service, or
  Postgres on your own machine (`docker run` below).
- **Nothing else.** No server, no queue, no AERO account (`PUBLISH=0` is the
  default and needs no token).

## Self-hosting on your own machine

### 1. Prerequisites

Rust 1.91+ (`rustup` works), `bun` for the front end, and Docker for Postgres
(or a Postgres you already run).

### 2. Database

```sh
docker run -d --name paperbro-db -p 5432:5432 \
  -e POSTGRES_USER=paperbro -e POSTGRES_PASSWORD=paperbro -e POSTGRES_DB=paperbro \
  postgres:16
```

Any Postgres works — a hosted free tier, your NAS, a Raspberry Pi. Migrations
run automatically at boot.

### 3. Configure

```sh
cp .env.example .env
```

Fill in `NEON_DB` (the name is a historical artifact — any Postgres works):

```sh
NEON_DB=postgresql://paperbro:paperbro@localhost:5432/paperbro
```

And the model you want. One provider, the "default" (old `LLM_*` names still
work):

```sh
LLM_BASE_URL=https://api.openai.com/v1
LLM_MODEL=gpt-4o
LLM_API_KEY=sk-…
```

Several providers, chosen by name; a request can name one, otherwise
`LLM_DEFAULT` is used:

```sh
LLM_DEFAULT=default

LLM_BASE_URL=https://api.openai.com/v1
LLM_MODEL=gpt-4o
LLM_API_KEY=sk-…
LLM_PRICE_INPUT=2.50
LLM_PRICE_CACHED=1.25
LLM_PRICE_OUTPUT=10.00

LLM_OLLAMA_BASE_URL=http://127.0.0.1:11434/v1
LLM_OLLAMA_MODEL=llama3.1:8b
LLM_OLLAMA_REASONING=0
LLM_OLLAMA_JSON_MODE=object
```

The server must stream usage (`stream_options.include_usage`) and support
strict `json_schema` output. Costs are counted at the per-provider
`LLM_<NAME>_PRICE_*` prices, so set them to what you actually pay; adjust
`DAILY_BUDGET_USD` accordingly. Use `https` for any server that isn't on this
machine: the key travels as a bearer token.

### 4. Build and run

```sh
mkdir -p .chrome && cd .chrome && \
  curl -sSLO https://storage.googleapis.com/chrome-for-testing-public/156.0.8078.47/mac-arm64/chrome-headless-shell-mac-arm64.zip && \
  unzip -q chrome-headless-shell-mac-arm64.zip && rm chrome-headless-shell-mac-arm64.zip && cd ..
(cd frontend && bun install && bun run build)
cargo run -- serve            # http://127.0.0.1:8083/preprint-bro/
```

That's it. Paste `1706.03762`, an `arxiv.org/abs/…` link, a
`biorxiv.org/content/…` link, a `medrxiv.org/content/…` link, or a bare
`10.1101/…` DOI (looked up on bioRxiv first, then medRxiv).

With `PUBLISH=0` a dry-run server starts no build workers unless `WORKERS` is
set, so the Build button only queues; `cargo run -- build <id>` builds one
course in the foreground. Finished courses are served at
`/preprint-bro/preview/<alias>`.

Other commands:

```sh
cargo test                        # unit tests
cargo run -- qa-sample            # browser QA on a course with deliberately broken labs
cargo run -- ingest 10.1101/2024.06.28.601293   # the text the model reads, written to tmp/
cargo run -- build 1706.03762     # build one course in the foreground (costs ~$1)
```

## Notes on bioRxiv / medRxiv

- Metadata comes from the public `api.biorxiv.org` API; full text is the
  paper's JATS XML (`*.source.xml`) from `biorxiv.org`/`medrxiv.org`.
- Those sites sit behind Cloudflare and rate-limit aggressively. preprint-bro
  fetches each paper's XML once, caches it, and retries politely, but if you
  build many courses in a row from one IP you may hit `429`s — space your
  builds out, or the queue keeps the paper and finishes later.
- Bare DOIs exist on both servers; preprint-bro tries bioRxiv first and falls
  back to medRxiv.

## Deploying to a server (optional)

The original deploy story still works unchanged; see the original README's
steps in git history. In short: build the front end, `cargo build --release`,
install the pinned `chrome-headless-shell` for linux64, set the environment,
and route `/preprint-bro*` on your public origin to `LISTEN`. `PUBLISH=1` needs
an `AERO_TOKEN` minted at `https://aero.oops.wtf/connect`.

### Forking for your own front end

By default the app serves its own files and sends no analytics anywhere. If
you fork and host the built `frontend/dist` on GitHub Pages, set
`ASSET_CDN=https://<you>.github.io/<repo>/preprint-bro/` to load static files
from the CDN first (falling back to your server), and `ANALYTICS_ID=G-…` to
inject your own GA4 tag. Leaving both empty is the self-hosted default.

### Operating

- **Ports:** `LISTEN` (8083) is the public listener; `ADMIN_LISTEN` (8085) is
  the admin listener, loopback only, no auth.
- **Spend:** set a budget on your model provider's account too.
  `DAILY_BUDGET_USD` is preprint-bro's own guard.
- **Updating a course:** `preprint-bro republish <id>` rebuilds a finished
  course's page from its stored lessons with the current page code, runs
  browser QA on it, and updates it in place. It makes no model calls and needs
  no `LLM_*` settings.

Admin, from the host itself:

```sh
curl -s 127.0.0.1:8085/jobs                       # every course and its state
curl -s 127.0.0.1:8085/events/1706.03762          # one course's history, incl. visitor reports
curl -s -X POST 127.0.0.1:8085/retry/1706.03762   # requeue a failed course
curl -s -X POST 127.0.0.1:8085/rebuild/1706.03762 # throw away checkpoints and rebuild
curl -s -X POST 127.0.0.1:8085/remove/1706.03762  # unpublish (takedown); never rebuilt
```

## Credits

A fork of [aeroxy/paper-bro](https://github.com/aeroxy/paper-bro) by aeroxy,
MIT licensed; upstream's AERO publishing flow is preserved behind
`PUBLISH=1`. preprint-bro isn't affiliated with arXiv, bioRxiv or medRxiv.
