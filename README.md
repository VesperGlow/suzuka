# 小凉花厨的网站 · Suzuka's Garden

[suzuka-chan.moe](https://suzuka-chan.moe/) 的源码。一个用 [Hugo](https://gohugo.io/) 搭建的双语个人站点，记录 Galgame 感想、偶尔浮现的思考，以及落下的生活碎片。

主题、布局与样式全部手写，没有引入外部主题；留言板、阅读量、点赞等动态内容由仓库内一个轻量的 Go + SQLite 后端提供。

## 特性

- **双语**：中文（默认）与 English，独立的菜单与文案（`i18n/`、`hugo.toml`）。
- **离线全文搜索**：构建期由 [Pagefind](https://pagefind.app/) 生成索引，无需后端即可搜索。
- **明暗主题**：跟随系统并可手动切换，配色用 CSS `light-dark()` 收敛。
- **动态交互**（依赖后端）：留言板、按文章统计的阅读量与点赞、关于页的站点总览与运行时长。
- **阅读体验**：文章目录侧栏、图片灯箱、返回顶部、归档时间线与标签/分类筛选。
- **订阅**：首页输出 RSS、JSON Feed 与一份给关于页用的 JSON 汇总。

## 技术栈

- Hugo（extended，v0.163+）静态站点
- 原生 HTML 模板 + CSS + 少量无框架 JavaScript（`assets/`、`layouts/`）
- Pagefind 搜索索引
- Go + `modernc.org/sqlite`（纯 Go 驱动，无需 CGO）后端，见 [`backend/`](backend/README.md)

## 目录结构

```
content/       文章、归档与独立页面（about / guestbook）
layouts/       手写的模板与 partials
assets/        CSS 与按需加载的 JS
i18n/          中英文案
static/        favicon、角色图等静态资源
hugo.toml      站点配置（语言、菜单、输出格式、permalink）
backend/       留言板 / 阅读量 / 点赞的 Go + SQLite 服务
```

## 本地开发

需要 Hugo **extended** 版本。

```sh
hugo server -D
```

留言板、阅读量等功能需要后端在本地一并运行，见 [`backend/README.md`](backend/README.md)。

## 构建

构建站点并生成 Pagefind 搜索索引：

```sh
hugo && npx -y pagefind --site public
```

等价于 `npm run build`。产物输出到 `public/`。

## 部署

前后端打进**一个容器**：Go 进程在根路径托管 Hugo 静态产物，并把留言板接口收敛到
`/api/guestbook/` 前缀下（与前端 `fetch` 路径一致，由进程自身剥前缀，无需 nginx 反代）。

- **自动构建**：push 到 `main` 时 GitHub Actions 跑 `go vet` + 测试，再三阶段构建
  （Hugo + Pagefind → 编译 Go → 合进 scratch）并推镜像到 GHCR
  `ghcr.io/<owner>/suzuka:latest`（见 `.github/workflows/site-image.yml` 与根目录 `Containerfile`）。
- **VPS 只负责跑容器**，监听 HTTP，由 Cloudflare / 反代在前面终止 TLS：

  ```sh
  # 命名卷持久化 SQLite，发布到宿主回环；外层代理把 80/443 转到 127.0.0.1:8787。
  podman pull   ghcr.io/<owner>/suzuka:latest
  podman run -d --name suzuka --restart=always \
    -p 127.0.0.1:8787:8787 \
    -v suzuka-data:/data \
    ghcr.io/<owner>/suzuka:latest
  ```

  Hugo 版本通过 `Containerfile` 的 `HUGO_VERSION` / `PAGEFIND_VERSION` 构建参数钉死，
  与本地保持一致。`backend/Containerfile` 仍保留，用于只跑后端 API 的场景。
