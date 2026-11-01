# Rill

**用 Rust 构建的轻量流式 API 代理。**

Rill 意为“小溪”。它把 OpenAI API 接到你自己的域名上：客户端保留原来的 OpenAI Key，只修改 Base URL。

[English](README.md) · 简体中文

```text
客户端 → 自定义域名 → Rill → OpenAI API
                   CF Workers / Rust 服务 / Docker
```

## 特点

- 一套 Rust 核心逻辑，两种运行方式：Cloudflare Workers WASM、Axum 原生服务器。
- 透传 `/v1/*`，覆盖 Responses、Chat Completions、模型列表等 HTTP API。
- 请求和响应直接流式转发，保留 SSE，不等待全部生成完毕。
- 保留上游状态码、错误正文、请求 ID、限流头和 OpenAI 项目/组织头。
- 提供 CORS 预检；不缓存模型响应，不记录 API Key、提示词或回复内容。
- 无数据库、计费后台和模型 SDK；服务端不保存 OpenAI Key。

当前版本支持 **HTTP / SSE**，不支持 WebSocket / Realtime，也不做 Claude/OpenAI 协议转换。调用费用仍由客户端所用 OpenAI Key 承担。可以访问实例的人都能用自己的 Key 转发请求；私用实例可在边缘或网络层限制访问。

## 使用

在客户端填写：

```text
Base URL: https://api.example.com/v1
API Key:  原来的 OpenAI API Key
```

OpenAI 上游是 `https://api.openai.com/v1`，不是 `/api/v1`。

```bash
curl -N https://api.example.com/v1/responses \
  -H "Authorization: Bearer $OPENAI_API_KEY" \
  -H 'Content-Type: application/json' \
  -d '{"model":"YOUR_MODEL","input":"你好","stream":true}'
```

## Cloudflare 部署

**免费 Workers 套餐即可部署**，无需数据库、KV 或其他付费服务。免费额度目前为每天 100,000 次请求、每次 10 ms CPU；等待上游和网络 I/O 不计入 CPU 时间，OpenAI 调用仍按自己的 Key 计费。见 [官方配额](https://developers.cloudflare.com/workers/platform/limits/)。

最简单的方式是下载已构建的包，只需要 Node.js 22+，不用安装 Rust：

```bash
curl -fL https://github.com/yzfly/rill/releases/download/v0.1.0/rill-worker.tar.gz | tar xz
cd rill-worker
npx wrangler@4.156.0 login
npx wrangler@4.156.0 deploy
```

也可以从源码构建：

```bash
rustup target add wasm32-unknown-unknown
cargo install worker-build --version 0.8.7 --locked
npm install -g wrangler
wrangler login
cd crates/worker
wrangler deploy
```

在 `crates/worker/wrangler.toml` 中添加自己的域名：

```toml
[[routes]]
pattern = "api.example.com"
custom_domain = true
```

业务逻辑全部用 Rust 编写；Cloudflare 工具生成必要的 JavaScript 入口。可以在构建机生成 `build/`，将它和 `wrangler.prebuilt.toml` 复制到发布机，再运行 `wrangler deploy --config wrangler.prebuilt.toml`。使用这种方式时，在该配置文件中填写自定义域名。

免费套餐受请求数和 CPU 配额限制。部署成功后还需从实际客户端网络测试带 Key 的请求和流式输出，确认 Cloudflare 出口与上游兼容。

如果 API 客户端收到 Cloudflare `1010`，可添加仅匹配 API 子域名的自定义规则，跳过 **Browser Integrity Check**。免费套餐可用，无需关闭整个域名的其他防护。见 [官方说明](https://developers.cloudflare.com/waf/tools/browser-integrity-check/)。

## 普通服务器 / Docker

Linux x86_64（Debian 12 或兼容系统）可直接下载单文件程序：

```bash
curl -fL https://github.com/yzfly/rill/releases/download/v0.1.0/rill-linux-x86_64.tar.gz | tar xz
./rill
```

其他系统从源码构建，Docker 为可选部署方式：

```bash
cargo build --locked --release -p rill-server
./target/release/rill
# 默认监听 127.0.0.1:8787，前置 Caddy / Nginx 提供 HTTPS。

# 或使用 Docker：
docker compose up --build -d
```

支持 `RILL_LISTEN`、容器平台常用的 `PORT`，以及指定上游源站的 `RILL_UPSTREAM`。上游默认 `https://api.openai.com`，不带 `/v1`；只有部署者能修改上游，客户端不能指定任意目标。

相同 Dockerfile 可作为 Fly.io、Railway、Render 等容器平台的部署入口；首版提供 Docker 配置，但尚未在 Docker 中构建运行，也未逐一验证这些平台。构建、配置、测试和限制见 [完整文档](README.md)。

## 长期部署选择

服务器版是普通 HTTP 服务，不依赖 Cloudflare。阿里云、腾讯云等 Linux 云主机可以直接运行原生程序，用 systemd 或 Docker 的重启策略维持服务；云函数平台则可能需要专门的入口适配。

| 选择 | 运行方式与限制 |
| --- | --- |
| Cloudflare Workers Free | 已验证，优先选择；使用专门的 WASM 构建包，受免费配额限制。 |
| 自有 Linux / VPS | 已验证原生程序；单文件运行，TLS 可交给现有反向代理。服务器和流量按原套餐计费。 |
| [Google Cloud Run](https://cloud.google.com/run/pricing) | 可部署容器，每月有持续免费额度；需要结算账号，超额、出站流量和镜像存储等可能收费。请求默认 5 分钟，最长可配置到 [60 分钟](https://docs.cloud.google.com/run/docs/configuring/request-timeout)。 |
| [Azure Container Apps](https://azure.microsoft.com/en-us/pricing/details/container-apps/) | Consumption 套餐每月有免费请求与计算额度，超额付费；默认 HTTP 入口有 [240 秒超时](https://learn.microsoft.com/en-us/azure/container-apps/ingress-overview)。 |
| [Oracle Always Free VM](https://docs.oracle.com/en-us/iaas/Content/FreeTier/freetier_topic-Always_Free_Resources.htm) | 可运行服务器版，但受容量限制，闲置免费实例可能被回收；ARM 机器需要重新构建，不能使用 x86_64 下载包。 |
| [Render Free](https://render.com/docs/free) | 可部署容器；15 分钟无请求会休眠，唤醒约需 1 分钟，不适合作为要求稳定响应的默认入口。 |

后三种容器平台和 Oracle 尚未逐平台部署验证。容器默认监听 `0.0.0.0:8787`，平台提供 `PORT` 时使用平台端口；健康检查路径为 `/health`。选择能访问上游的部署地区，并实际验证 SSE。容器平台按资源分配时间计费，长时间等待模型也可能消耗免费计算额度。

## 资源占用

0.1.0 在 Linux x86_64 上的测量：程序约 **3.35 MiB**，空闲 RSS 约 **3.9 MiB**；4 路并发转发各 8 MiB 的本地 HTTP 数据时，RSS 峰值约 **4.9 MiB**。使用单线程异步运行时，实际占用随 TLS、并发和平台变化，这组数据不代表容量保证。

Cloudflare 上传包约 **524 KiB**，gzip 后约 **162 KiB**，部署报告启动时间 **3 ms**。启动时间不是单次请求 CPU 时间。

设计参考 [x-dr/chatgptProxyAPI](https://github.com/x-dr/chatgptProxyAPI) 的简洁转发思路。MIT 开源。
