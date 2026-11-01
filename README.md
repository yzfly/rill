# Rill

**A self-hosted OpenAI API reverse proxy in Rust. Callers change one setting, the Base URL, and nothing else.**

Rill puts the OpenAI API behind your own domain. Chat Completions, the Responses API, Embeddings, Decisions and every other `/v1/*` endpoint pass through unchanged, and streaming (SSE) arrives token by token. Callers keep their own API key and need no new SDK, plugin or local process. Deploy it on **Cloudflare Workers Free** or as a **single ~3 MiB binary**.

English · [简体中文](README.zh-CN.md)

```text
Your app / SDK / tool ──Base URL──▶ https://api.example.com/v1 (Rill) ──▶ https://api.openai.com/v1
                                     Cloudflare Workers · Linux binary · Docker
```

## Is this what you are looking for?

Rill solves problems usually described like this:

- "Put the OpenAI API behind my own domain / custom `base_url`."
- "A Cloudflare Worker that proxies OpenAI, with streaming that actually works."
- "Self-hosted OpenAI relay where clients only change `OPENAI_BASE_URL`."
- "A reverse proxy for `/v1/chat/completions` and `/v1/responses` that does not buffer SSE."
- "A small OpenAI gateway with no database or dashboard that never stores keys."
- "One fixed endpoint for my team, scripts and tools instead of hard-coding `api.openai.com`."

Rill is **not** a model router, protocol translator (Claude ↔ OpenAI), key pool, billing system or usage dashboard. If you need those, look at a full API gateway instead.

## Use it: change only the Base URL

| Setting | Value |
| --- | --- |
| Base URL / API Base / Endpoint | `https://api.example.com/v1` |
| API key | Your existing OpenAI API key, unchanged |

Official SDKs read the base URL from the environment, so most programs need no code change:

```bash
export OPENAI_BASE_URL=https://api.example.com/v1
export OPENAI_API_KEY=sk-...        # your own key, as before
```

```python
from openai import OpenAI
client = OpenAI(base_url="https://api.example.com/v1")   # or rely on OPENAI_BASE_URL

client.chat.completions.create(model="gpt-4.1-mini", messages=[{"role": "user", "content": "Hello"}])
client.responses.create(model="gpt-4.1-mini", input="Hello")
client.embeddings.create(model="text-embedding-3-small", input="Hello")
```

```js
import OpenAI from "openai";
const client = new OpenAI({ baseURL: "https://api.example.com/v1" });
```

```bash
# Streaming Responses API through Rill
curl -N https://api.example.com/v1/responses \
  -H "Authorization: Bearer $OPENAI_API_KEY" \
  -H 'Content-Type: application/json' \
  -d '{"model":"gpt-4.1-mini","input":"Say hello","stream":true}'
```

Any tool with an "OpenAI-compatible endpoint", "API Base" or "Base URL" field works the same way. Enter the URL **with** `/v1`.

## Supported APIs

Rill forwards every `/v1/*` path, method, query string and body byte unchanged. It does not parse or rewrite JSON, so new OpenAI endpoints and parameters work without a Rill update.

| API | Path | Status |
| --- | --- | --- |
| Chat Completions (incl. `stream: true`) | `/v1/chat/completions` | ✅ Verified with a real key, plain and SSE |
| Responses API (incl. streaming events) | `/v1/responses`, `/v1/responses/{id}` | ✅ Verified with a real key, plain and SSE |
| Embeddings | `/v1/embeddings` | ✅ Verified with a real key |
| Decisions (`gpt-6-luna`: predicate / choice / score answers) | `/v1/decisions` | ✅ Verified with a real key |
| Models | `/v1/models` | ✅ Verified with a real key |
| Error pass-through (4xx/5xx bodies, `x-request-id`, rate-limit headers) | any | ✅ Verified |
| Legacy Completions, Images, Audio (speech, transcription uploads), Files, Uploads, Batch, Moderations, Vector Stores, Assistants, Fine-tuning | `/v1/...` | Forwarded as-is; not individually tested |
| Realtime API over WebSocket | `/v1/realtime` | ❌ Not supported (HTTP/SSE only) |

The verified rows were run on 2026-11-01 against a Cloudflare Workers deployment using the official `openai` Python SDK 3.42.0 and curl. Only `OPENAI_BASE_URL` was changed.

Requests must use `Authorization: Bearer <key>`. Header-based schemes like Azure OpenAI's `api-key` or Anthropic's `x-api-key` are rejected.

## What Rill guarantees

- **Streaming end to end.** Request and response bodies are never collected in memory, and SSE events reach the client as OpenAI emits them.
- **Faithful responses.** Upstream status codes, error JSON, `x-request-id`, `openai-processing-ms` and `x-ratelimit-*` headers are preserved and exposed to browsers via CORS.
- **Bring your own key.** The caller's key is billed. Rill stores no provider key, and its code logs no keys, prompts or completions.
- **Fixed upstream.** Only the operator sets the upstream origin. Clients cannot point Rill at arbitrary hosts, and `..` / `%2e%2e` paths cannot escape `/v1/`. Upstream redirects are returned, never followed with the key.
- **Clean forwarding.** Cookies, `Origin`/`Referer`, `X-Forwarded-*`, `CF-*` and hop-by-hop headers are dropped. Responses are `Cache-Control: no-store`.
- **Browser-ready.** CORS preflight is answered locally. `OPTIONS` never reaches OpenAI.

## Deploy on Cloudflare Workers (free plan)

No database, KV or paid add-on is required. Workers Free currently allows 100,000 requests/day and 10 ms CPU per request. Time spent waiting on OpenAI is not CPU time ([limits](https://developers.cloudflare.com/workers/platform/limits/)).

Prebuilt bundle, Node.js 22+, no Rust needed:

```bash
curl -fL https://github.com/yzfly/rill/releases/download/v0.1.0/rill-worker.tar.gz | tar xz
cd rill-worker
npx wrangler@4.156.0 login
npx wrangler@4.156.0 deploy
# → https://rill.<your-subdomain>.workers.dev  (Base URL: .../v1)
```

From source:

```bash
rustup target add wasm32-unknown-unknown
cargo install worker-build --version 0.8.7 --locked
npm install -g wrangler && wrangler login
cd crates/worker
wrangler dev        # local test
wrangler deploy
```

**Use your own domain.** `*.workers.dev` is unreachable from some networks, and a custom domain is easier to share. After adding the domain to Cloudflare, put this in `crates/worker/wrangler.toml`, or in a git-ignored `wrangler.production.toml`:

```toml
[[routes]]
pattern = "api.example.com"
custom_domain = true
```

Set `workers_dev = false` once the custom domain works. To deploy a build made on another machine, copy `crates/worker/build/` and `wrangler.prebuilt.toml`, then run `wrangler deploy --config wrangler.prebuilt.toml`.

**Region rule on Workers (measured).** OpenAI only serves [supported countries and territories](https://platform.openai.com/docs/supported-countries). Through a Worker, OpenAI applies this to the **original caller's location**, not the Cloudflare data center. The same deployment returned the normal response to a US caller and `403 unsupported_country_region_territory` to a caller in an unsupported region, even though both reached the same US data center. Rill does not change who may use the API. Callers must be in a supported region and follow OpenAI's usage policies.

All proxy logic is Rust compiled to WebAssembly. The small JavaScript entrypoint is generated by Cloudflare's `worker-build`.

## Run as a standalone server

Linux x86_64 (Debian 12 or compatible):

```bash
curl -fL https://github.com/yzfly/rill/releases/download/v0.1.0/rill-linux-x86_64.tar.gz | tar xz
./rill                       # listens on 127.0.0.1:8787
```

Or build anywhere Rust runs (Linux ARM, macOS, …):

```bash
cargo build --locked --release -p rill-server
./target/release/rill
```

Put it behind your HTTPS reverse proxy and **turn off response buffering** so SSE streams:

```caddyfile
api.example.com {
    reverse_proxy 127.0.0.1:8787 {
        flush_interval -1
    }
}
```

```nginx
location /v1/ {
    proxy_pass http://127.0.0.1:8787;
    proxy_http_version 1.1;
    proxy_buffering off;
    proxy_read_timeout 3600s;
}
```

Docker:

```bash
docker compose up --build -d
curl http://127.0.0.1:8787/health
```

The image runs as a non-root user and Compose binds to loopback. The Docker files ship with this release but have not yet been built and run in Docker.

| Variable | Default | Meaning |
| --- | --- | --- |
| `RILL_UPSTREAM` | `https://api.openai.com` | Upstream **origin** only: no `/v1`, credentials, query or fragment. Works for both server and Worker. |
| `RILL_LISTEN` | `127.0.0.1:8787` | Listen address (server only). |
| `PORT` | none | If set and `RILL_LISTEN` is not, binds `0.0.0.0:$PORT` (container platforms). |
| `HTTPS_PROXY` / `NO_PROXY` | none | Outbound proxy for the server's own upstream connection, if your network requires one. |

`RILL_UPSTREAM` can point at any service that speaks the OpenAI API under `/v1` at its origin root. Only use an upstream you trust with your callers' keys. Plain HTTP is meant for local tests.

The server shuts down gracefully on Ctrl+C / SIGTERM. It has a 15-second connect timeout and no whole-response timeout, so long generations are never cut off. It never retries or follows redirects. Upstream connection failures return a `502` JSON error and log one line to stderr, without URL, headers or bodies.

### Where to host

| Platform | Notes |
| --- | --- |
| Cloudflare Workers Free | Verified. Simplest default within the free quotas. |
| Your Linux server / VPS | Native binary verified. Use your existing HTTPS reverse proxy. |
| [Google Cloud Run](https://cloud.google.com/run/pricing) | Container with recurring free quota; billing account required. Requests default to 5 min, configurable up to [60 min](https://docs.cloud.google.com/run/docs/configuring/request-timeout). |
| [Azure Container Apps](https://azure.microsoft.com/en-us/pricing/details/container-apps/) | Consumption plan has free allowances. HTTP ingress has a [240 s timeout](https://learn.microsoft.com/en-us/azure/container-apps/ingress-overview). |
| [Oracle Always Free VM](https://docs.oracle.com/en-us/iaas/Content/FreeTier/freetier_topic-Always_Free_Resources.htm) | Subject to capacity and idle reclamation. ARM needs a source build. |
| [Render Free](https://render.com/docs/free) | Sleeps after 15 idle minutes, about 1 min to wake. Fine for experiments. |

Cloud Run, Container Apps, Oracle and Render have not been individually deployed for this release. Use `/health` for health checks and verify SSE from your real client network.

## Troubleshooting

| Symptom | Cause and fix |
| --- | --- |
| `404 Use /v1/* for API requests.` | The Base URL is missing `/v1`, or the client added a second `/v1`. Use exactly `https://api.example.com/v1`. |
| `401 Supply your OpenAI API key as Authorization: Bearer <key>.` | Returned by Rill: no Bearer key was sent. A `401` with OpenAI's wording means the key itself is wrong. |
| `403 unsupported_country_region_territory` | Returned by OpenAI, not Rill: the caller's region is not supported. On Workers, the caller's location counts (see above). |
| Cloudflare error `1010` | Browser Integrity Check is blocking an API client. Add a custom rule for the API hostname that skips **Browser Integrity Check** only ([docs](https://developers.cloudflare.com/waf/tools/browser-integrity-check/)). |
| Streaming arrives all at once | A reverse proxy in front of the server is buffering. Use `flush_interval -1` (Caddy) or `proxy_buffering off` (nginx). |
| `*.workers.dev` times out | Some networks block that domain. Bind a custom domain. |
| `501 unsupported_transport` | A WebSocket (Realtime) upgrade was attempted. Use HTTP/SSE endpoints. |

## Security and scope

- Anyone who can reach your instance can use it **with their own valid key**. For a private instance, restrict access at the network or edge (Cloudflare Access, WAF rate limits, firewall). Rill has no per-user quotas.
- Keep API keys in client configuration. Never ship a key in a public web page or repository.
- Rill keeps no response cache and no request or body logs. Your hosting provider may keep its own logs, so run an instance you trust.

## Footprint

Measured on 0.1.0, Linux x86_64: **3.35 MiB** binary, about **3.9 MiB** idle RSS, **4.9 MiB** peak RSS while forwarding four concurrent 8 MiB streams. Single-threaded async runtime. These numbers are not a capacity guarantee. The Worker upload is about **525 KiB** (**167 KiB** gzipped) with a **3–5 ms** startup.

## Development

```bash
cargo fmt --all --check
cargo test --locked
cargo clippy --locked -p rill-core -p rill-server --all-targets -- -D warnings
cargo check --locked -p rill-worker --target wasm32-unknown-unknown
(cd crates/worker && worker-build --release)
node scripts/smoke-worker.mjs      # runs the WASM bundle in local workerd
```

The tests use local mock upstreams, with no API key and no paid requests. They cover byte-exact forwarding, incremental SSE, auth and preflight, error and header preservation, redirects and path-escape attempts.

| Crate | Responsibility |
| --- | --- |
| `rill-core` | Transport-independent routing, auth and header policy (`prepare` → `Decision::Local` / `Decision::Forward`) |
| `rill-server` | Axum + Reqwest transport |
| `rill-worker` | Cloudflare Workers transport (workers-rs) |

MIT license.
