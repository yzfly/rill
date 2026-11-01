# Rill

**用 Rust 写的自托管 OpenAI API 反向代理：调用端只改一个 Base URL，其他什么都不用动。**

Rill 把 OpenAI API 接到你自己的域名上。Chat Completions、Responses API、Embeddings、Decisions 以及其他所有 `/v1/*` 接口原样透传，流式输出（SSE）逐 token 到达。调用端保留自己的 API Key，不用装 SDK、插件或本地服务。可部署在 **Cloudflare Workers 免费套餐**，也可以作为一个 **约 3 MiB 的单文件程序**运行。

[English](README.md) · 简体中文

```text
你的应用 / SDK / 工具 ──Base URL──▶ https://api.example.com/v1（Rill）──▶ https://api.openai.com/v1
                                     Cloudflare Workers · Linux 程序 · Docker
```

## 你是不是在找这个？

Rill 解决的通常是这类需求：

- “OpenAI API 反向代理 / 中转，用自己的域名做 `base_url`”
- “用 Cloudflare Workers 代理 OpenAI 接口，并且流式输出正常”
- “自建 OpenAI 中转，客户端只改 `OPENAI_BASE_URL` 就能用”
- “代理 `/v1/chat/completions` 和 `/v1/responses`，SSE 不被缓冲”
- “不要数据库、不要后台、不保存 Key 的轻量 OpenAI 网关”
- “给团队、脚本和工具一个统一入口，不再写死 `api.openai.com`”

Rill **不是**模型路由、协议转换（Claude ↔ OpenAI）、Key 池、计费系统或用量后台。需要这些功能请选择完整的 API 网关。

## 使用：只改 Base URL

| 配置项 | 填写 |
| --- | --- |
| Base URL / API Base / 接口地址 | `https://api.example.com/v1` |
| API Key | 原来的 OpenAI API Key，不变 |

官方 SDK 会读取环境变量，大多数程序不用改代码：

```bash
export OPENAI_BASE_URL=https://api.example.com/v1
export OPENAI_API_KEY=sk-...        # 仍是你自己的 Key
```

```python
from openai import OpenAI
client = OpenAI(base_url="https://api.example.com/v1")   # 或直接依赖 OPENAI_BASE_URL

client.chat.completions.create(model="gpt-4.1-mini", messages=[{"role": "user", "content": "你好"}])
client.responses.create(model="gpt-4.1-mini", input="你好")
client.embeddings.create(model="text-embedding-3-small", input="你好")
```

```js
import OpenAI from "openai";
const client = new OpenAI({ baseURL: "https://api.example.com/v1" });
```

```bash
# 经 Rill 调用流式 Responses API
curl -N https://api.example.com/v1/responses \
  -H "Authorization: Bearer $OPENAI_API_KEY" \
  -H 'Content-Type: application/json' \
  -d '{"model":"gpt-4.1-mini","input":"你好","stream":true}'
```

任何带有“OpenAI 兼容接口”“API Base”“Base URL”设置项的工具都一样使用。地址要**带上** `/v1`。

## 支持的接口

Rill 原样转发所有 `/v1/*` 的路径、方法、查询参数和请求体字节，不解析也不改写 JSON。OpenAI 新增的接口和参数不需要升级 Rill 就能用。

| 接口 | 路径 | 状态 |
| --- | --- | --- |
| Chat Completions（含 `stream: true`） | `/v1/chat/completions` | ✅ 真实 Key 实测，普通与 SSE |
| Responses API（含流式事件） | `/v1/responses`、`/v1/responses/{id}` | ✅ 真实 Key 实测，普通与 SSE |
| Embeddings 向量 | `/v1/embeddings` | ✅ 真实 Key 实测 |
| Decisions 决策（`gpt-6-luna`：是非 / 选择 / 打分题） | `/v1/decisions` | ✅ 真实 Key 实测 |
| 模型列表 | `/v1/models` | ✅ 真实 Key 实测 |
| 错误透传（4xx/5xx 正文、`x-request-id`、限流头） | 全部 | ✅ 实测 |
| 旧版 Completions、Images、Audio（语音合成、转写上传）、Files、Uploads、Batch、Moderations、Vector Stores、Assistants、Fine-tuning | `/v1/...` | 原样转发，未逐项实测 |
| Realtime API（WebSocket） | `/v1/realtime` | ❌ 不支持（仅 HTTP / SSE） |
| 非 OpenAI 路径，如火山方舟 System One `/api/v1/systemone` | `/v1/` 以外 | ❌ 不转发：Rill 只代理源站的 `/v1/*` |

实测于 2026-11-01，经 Cloudflare Workers 部署，使用官方 `openai` Python SDK 3.42.0 和 curl，只设置了 `OPENAI_BASE_URL`。

请求须使用 `Authorization: Bearer <key>`。Azure OpenAI 的 `api-key`、Anthropic 的 `x-api-key` 等头部鉴权会被拒绝。

## Rill 保证什么

- **全程流式**：请求和响应体不在内存中攒齐，SSE 事件随 OpenAI 输出实时到达。
- **响应保真**：保留上游状态码、错误 JSON、`x-request-id`、`openai-processing-ms` 和 `x-ratelimit-*` 头，并通过 CORS 暴露给浏览器。
- **自带 Key**：费用由调用者自己的 Key 承担。Rill 不保存任何 Key，代码不记录 Key、提示词或回复内容。
- **上游固定**：只有部署者能设置上游源站。客户端无法让 Rill 访问任意主机，`..` / `%2e%2e` 也逃不出 `/v1/`。上游重定向原样返回，不会带着 Key 跟随。
- **干净转发**：丢弃 Cookie、`Origin`/`Referer`、`X-Forwarded-*`、`CF-*` 和逐跳头。响应带 `Cache-Control: no-store`。
- **浏览器可用**：CORS 预检在本地应答，`OPTIONS` 不会发到 OpenAI。

## 部署到 Cloudflare Workers（免费套餐）

无需数据库、KV 或付费服务。免费套餐目前每天 100,000 次请求、每次 10 ms CPU，等待 OpenAI 的时间不计入 CPU（[官方配额](https://developers.cloudflare.com/workers/platform/limits/)）。

使用已构建的包，只需 Node.js 22+，不用装 Rust：

```bash
curl -fL https://github.com/yzfly/rill/releases/download/v0.1.0/rill-worker.tar.gz | tar xz
cd rill-worker
npx wrangler@4.156.0 login
npx wrangler@4.156.0 deploy
# → https://rill.<你的子域>.workers.dev（Base URL 填 .../v1）
```

从源码构建：

```bash
rustup target add wasm32-unknown-unknown
cargo install worker-build --version 0.8.7 --locked
npm install -g wrangler && wrangler login
cd crates/worker
wrangler dev        # 本地测试
wrangler deploy
```

**绑定自己的域名。** `*.workers.dev` 在部分网络下无法访问，自定义域名也更便于分享。域名接入 Cloudflare 后，在 `crates/worker/wrangler.toml`（或不入 Git 的 `wrangler.production.toml`）中添加：

```toml
[[routes]]
pattern = "api.example.com"
custom_domain = true
```

自定义域名可用后，可以设置 `workers_dev = false`。在另一台机器构建时，复制 `crates/worker/build/` 和 `wrangler.prebuilt.toml`，运行 `wrangler deploy --config wrangler.prebuilt.toml`。

**Workers 上的地区规则（实测）。** OpenAI 只服务[支持的国家和地区](https://platform.openai.com/docs/supported-countries)。经 Worker 转发时，OpenAI 依据的是**原始调用者所在地**，而不是 Cloudflare 机房。同一个部署，美国调用者收到正常响应；不受支持地区的调用者即使连到同一个美国机房，也收到 `403 unsupported_country_region_territory`。Rill 不改变谁可以使用 API：调用者须位于支持地区，并遵守 OpenAI 的使用政策。

代理逻辑全部是 Rust，编译为 WebAssembly；入口的少量 JavaScript 由 Cloudflare 的 `worker-build` 生成。

## 普通服务器 / Docker

Linux x86_64（Debian 12 或兼容系统）：

```bash
curl -fL https://github.com/yzfly/rill/releases/download/v0.1.0/rill-linux-x86_64.tar.gz | tar xz
./rill                       # 监听 127.0.0.1:8787
```

其他平台（Linux ARM、macOS 等）从源码构建：

```bash
cargo build --locked --release -p rill-server
./target/release/rill
```

前面放 HTTPS 反向代理，并**关闭响应缓冲**，否则流式输出会被攒到最后才返回：

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

Docker：

```bash
docker compose up --build -d
curl http://127.0.0.1:8787/health
```

镜像以非 root 用户运行，Compose 只绑定回环地址。Docker 配置随本版提供，但尚未在 Docker 中实际构建运行。

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `RILL_UPSTREAM` | `https://api.openai.com` | 只填上游**源站**：不带 `/v1`、账号、查询或片段。服务器和 Worker 通用。 |
| `RILL_LISTEN` | `127.0.0.1:8787` | 监听地址（仅服务器）。 |
| `PORT` | 无 | 设置且未设 `RILL_LISTEN` 时监听 `0.0.0.0:$PORT`（容器平台）。 |
| `HTTPS_PROXY` / `NO_PROXY` | 无 | 服务器自身访问上游时使用的出站代理，网络环境需要时使用。 |

`RILL_UPSTREAM` 可以指向任何在源站根路径 `/v1` 下提供 OpenAI 格式接口的服务。只填写你信任、可以交付调用者 Key 的上游；HTTP 仅用于本地测试。

服务器支持 Ctrl+C / SIGTERM 优雅退出。连接超时 15 秒，不设整体响应超时，长时间生成不会被截断。不自动重试，不跟随重定向。连不上上游时返回 `502` JSON 错误，并向 stderr 记一行日志，不含 URL、头部和正文。

### 托管选择

| 平台 | 说明 |
| --- | --- |
| Cloudflare Workers Free | 已验证，免费额度内最简单的默认选择。 |
| 自有 Linux 服务器 / VPS | 原生程序已验证，TLS 交给现有反向代理。 |
| [Google Cloud Run](https://cloud.google.com/run/pricing) | 容器，每月有免费额度，需要结算账号。请求默认 5 分钟，最长可配置 [60 分钟](https://docs.cloud.google.com/run/docs/configuring/request-timeout)。 |
| [Azure Container Apps](https://azure.microsoft.com/en-us/pricing/details/container-apps/) | Consumption 套餐有免费额度，HTTP 入口 [240 秒超时](https://learn.microsoft.com/en-us/azure/container-apps/ingress-overview)。 |
| [Oracle Always Free VM](https://docs.oracle.com/en-us/iaas/Content/FreeTier/freetier_topic-Always_Free_Resources.htm) | 受容量限制，闲置实例可能被回收；ARM 需从源码构建。 |
| [Render Free](https://render.com/docs/free) | 15 分钟无请求会休眠，唤醒约 1 分钟，适合试验。 |

Cloud Run、Container Apps、Oracle、Render 本版未逐一部署验证。健康检查用 `/health`，并从真实客户端网络验证 SSE。

## 常见问题

| 现象 | 原因与处理 |
| --- | --- |
| `404 Use /v1/* for API requests.` | Base URL 少了 `/v1`，或客户端又拼了一次 `/v1`。填写 `https://api.example.com/v1`。 |
| `401 Supply your OpenAI API key as Authorization: Bearer <key>.` | Rill 返回：请求没带 Bearer Key。如果 401 是 OpenAI 的提示文字，说明 Key 本身不对。 |
| `403 unsupported_country_region_territory` | OpenAI 返回，不是 Rill：调用者所在地区不受支持。Workers 上以调用者所在地为准（见上文）。 |
| Cloudflare 错误 `1010` | 浏览器完整性检查拦截了 API 客户端。为 API 子域名添加自定义规则，只跳过 **Browser Integrity Check**（[说明](https://developers.cloudflare.com/waf/tools/browser-integrity-check/)）。 |
| 流式输出最后一次性到达 | 前置反向代理在缓冲。Caddy 用 `flush_interval -1`，nginx 用 `proxy_buffering off`。 |
| `*.workers.dev` 超时 | 部分网络屏蔽该域名，绑定自定义域名即可。 |
| `501 unsupported_transport` | 尝试了 WebSocket（Realtime）升级，请改用 HTTP / SSE 接口。 |

## 安全与边界

- 能访问实例的人都可以**用自己的有效 Key** 转发请求。私用实例请在网络或边缘限制访问（Cloudflare Access、WAF 限速、防火墙）。Rill 没有按用户的配额。
- Key 只放在调用端配置里，不要写进公开网页或仓库。
- Rill 不缓存响应，不记录请求和正文。托管平台可能有自己的日志，请使用你信任的实例。

## 资源占用

0.1.0 在 Linux x86_64 上测得：程序 **3.35 MiB**，空闲 RSS 约 **3.9 MiB**，4 路并发各转发 8 MiB 时峰值 **4.9 MiB**。使用单线程异步运行时，数据不代表容量保证。Worker 上传包约 **525 KiB**（gzip 后 **167 KiB**），启动 **3–5 ms**。

## 开发

```bash
cargo fmt --all --check
cargo test --locked
cargo clippy --locked -p rill-core -p rill-server --all-targets -- -D warnings
cargo check --locked -p rill-worker --target wasm32-unknown-unknown
(cd crates/worker && worker-build --release)
node scripts/smoke-worker.mjs      # 在本地 workerd 中运行 WASM 包
```

测试使用本地模拟上游，不需要 API Key，也不产生费用。覆盖逐字节转发、增量 SSE、鉴权与预检、错误和头部保留、重定向和路径逃逸。

| Crate | 职责 |
| --- | --- |
| `rill-core` | 与传输无关的路由、鉴权和头部策略（`prepare` → `Decision::Local` / `Decision::Forward`） |
| `rill-server` | Axum + Reqwest 传输 |
| `rill-worker` | Cloudflare Workers 传输（workers-rs） |

MIT 开源。
