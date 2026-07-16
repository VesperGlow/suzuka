# 小凉花厨的网站 · Suzuka's Garden

[suzuka-chan.moe](https://suzuka-chan.moe/) 的源码。一个双语个人站点，记录 Galgame 感想、偶尔浮现的思考，以及落下的生活碎片。静态站由仓库内自研的 Rust 生成器（[`ssg/`](ssg/README.md)）构建，此前是 [Hugo](https://gohugo.io/) 站点迁移而来。

主题、布局与样式全部手写，没有引入外部主题；留言板、阅读量、点赞等动态内容由仓库内一个轻量的 Rust + SQLite 后端提供。

## 特性

- **双语**：中文（默认）与 English，独立的菜单与文案（`i18n/`、`site.toml`）。
- **离线全文搜索**：构建期由 [Pagefind](https://pagefind.app/) 生成索引，无需后端即可搜索。
- **明暗主题**：跟随系统并可手动切换，配色用 CSS `light-dark()` 收敛。
- **动态交互**（依赖后端）：留言板、按文章统计的阅读量与点赞、关于页的站点总览与运行时长。
- **阅读体验**：文章目录侧栏、图片灯箱、返回顶部、归档时间线与标签/分类筛选。
- **订阅**：首页输出 RSS、JSON Feed 与一份给关于页用的 JSON 汇总。

## 技术栈

- 自研 Rust 静态站点生成器（[`ssg/`](ssg/README.md)），单站定制，非通用 SSG
- 原生 HTML 模板 + CSS + 少量无框架 JavaScript（`assets/`、`ssg/templates/`）
- Pagefind 搜索索引
- Rust（axum + rusqlite，bundled SQLite 随 crate 静态编译）后端，见 [`backend/`](backend/README.md)

## 目录结构

```
content/       文章、归档与独立页面（about / guestbook）
ssg/           自研静态站点生成器（Rust），模板在 ssg/templates/
assets/        CSS 与按需加载的 JS
i18n/          中英文案
static/        favicon、角色图等静态资源
site.toml     站点配置（语言、菜单、输出格式、permalink），ssg 自己在读的配置格式
backend/       留言板 / 阅读量 / 点赞的 Rust + SQLite 服务
```

## 本地开发

```sh
# 热重载预览：构建 + 监听变更自动重建 + 内置静态服务器
cargo run --manifest-path ssg/Cargo.toml -- serve --source .
# 然后浏览 http://127.0.0.1:1313/（--addr 可改端口，--dest 默认 public-dev）
```

`serve` 不跑 Pagefind（本地搜索不可用）也不 minify；一次性构建照旧：

```sh
cargo run --manifest-path ssg/Cargo.toml -- build --source . --dest public
npx -y pagefind@1.3.0 --site public
```

留言板、阅读量等功能需要后端在本地一并运行。注意前端 `fetch` 的路径写死在
`/api/guestbook/` 前缀下，而这个前缀只在后端的**单容器模式**（设置了
`GUESTBOOK_STATIC_DIR`）下存在——用独立静态服务器预览时这些请求会 404。
本地联调动态功能请让后端一体托管静态产物：

```sh
GUESTBOOK_STATIC_DIR=public cargo run --manifest-path backend/Cargo.toml
# 然后浏览 http://127.0.0.1:8787/
```

详见 [`backend/README.md`](backend/README.md) 的两种路由模式说明。

## 构建

构建站点并生成 Pagefind 搜索索引：

```sh
npm run build
```

等价于 `cargo build --release --locked --manifest-path ssg/Cargo.toml && rm -rf public && ./ssg/target/release/ssg build --source . --dest public --minify && npx -y pagefind@1.3.0 --site public`。
产物输出到 `public/`。

## 部署

前后端打进**一个容器**：后端进程在根路径托管 ssg 静态产物，并把留言板接口收敛到
`/api/guestbook/` 前缀下（与前端 `fetch` 路径一致，由进程自身剥前缀，无需 nginx 反代）。

- **自动构建**：push 到 `main` 时 GitHub Actions 跑 `cargo test`（backend）、`cargo build`
  与全站构建冒烟 + 站内死链检查（ssg）；clippy / rustfmt 两边都是阻断项。全部通过后
  再三阶段构建（ssg 生成静态站 + Pagefind 索引 → 编译 Rust 后端 → 合进 scratch）并推镜像到
  GHCR `ghcr.io/<owner>/suzuka:latest`（见 `.github/workflows/site-image.yml` 与根目录
  `Containerfile`）。
- **VPS 只负责跑容器**，监听 HTTP，由 Cloudflare / 反代在前面终止 TLS：

  ```sh
  # 命名卷持久化 SQLite，发布到宿主回环；外层代理把 80/443 转到 127.0.0.1:8787。
  # GUESTBOOK_ADMIN_TOKEN 可选：设置后开启留言删除接口（无 WebUI，curl 管理，
  # 见 backend/README.md），不设置则后端没有任何管理面。
  # GUESTBOOK_SMTP_USER/PASSWORD 可选：设置后新留言会发邮件通知（Gmail 需要
  # App Password，见 backend/README.md），不设置则不发信。
  podman pull   ghcr.io/<owner>/suzuka:latest
  podman run -d --name suzuka --restart=always \
    -p 127.0.0.1:8787:8787 \
    -v suzuka-data:/data \
    -e GUESTBOOK_ADMIN_TOKEN=<随机长串> \
    -e GUESTBOOK_SMTP_USER=<你的 Gmail 地址> \
    -e GUESTBOOK_SMTP_PASSWORD=<Gmail App Password> \
    ghcr.io/<owner>/suzuka:latest

  # 删除一条垃圾留言（id 从留言板页面或 GET /api/guestbook/messages 里看）：
  curl -X DELETE -H "Authorization: Bearer <随机长串>" \
    https://suzuka-chan.moe/api/guestbook/messages/<id>
  ```

  外层反代必须覆盖客户端传入的 `X-Forwarded-For`，不要原样追加。留言数据位于
  `/data` 卷；后端每天自动在 `/data/backups/` 落一份 `VACUUM INTO` 的一致性
  快照（保留最近 7 份），**直接复制 / rsync 整个数据目录即可完成备份**——
  运行中的 `guestbook.db` 本体（连同 -wal/-shm）拷出来可能缺数据，但快照
  文件永远是完整可用的数据库。

  Pagefind 版本通过 `Containerfile` 的 `PAGEFIND_VERSION` 构建参数钉死，与本地保持一致。
  前后端已经合并进同一个容器，不再单独提供只跑后端 API 的 Containerfile。
