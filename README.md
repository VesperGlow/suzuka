# 文字庭院

[0721.ing](https://0721.ing/) 的源码。一个双语个人静态博客，记录阅读感受、日常思考与生活片段。

Markdown、HTML 模板、CSS 和原生 JavaScript 由仓库内的 [Rust 构建器](ssg/README.md) 生成到 `public/`。部署只需托管静态文件，无需应用后端或数据库。

## 功能

- 中文与 English 页面，共用「文字庭院」这一站名。
- 目录式首页、图文与时间线归档、分类/标签筛选、文章目录、图片灯箱和返回顶部。
- 桌面使用居中的顶部导航，窄屏通过顶栏右侧的菜单按钮展开单行横向导航；最大 1200px 内容区，文章正文单独限制在 820px；桌面正文 18px，移动端保留原有字号。标题使用本机中文衬线字体，正文使用无衬线字体，无需加载外部字体资源。
- 跟随系统或手动切换明暗主题。
- 中文使用静态 JSON 索引搜索；英文优先使用 Pagefind，也支持 JSON 搜索回退。
- RSS 与 JSON Feed 订阅，仅收录文章。
- 关于页的文章数和字数在构建期生成，记录时长从最早发表的文章开始计算。

站点不使用头像、人物立绘或默认分享图。文章有配图时，分享预览使用文章自身的图片；无图页面只输出文字元数据。

正文、列表、导航和统计均直接存在于 HTML 中；JavaScript 用于浏览器交互，搜索只读取站内静态文件。

## 开发与构建

需要 Rust（版本见 `rust-toolchain.toml`）、Node.js / npm；检查脚本使用 Python 3.11 或更新版本。

```sh
npm run dev       # http://127.0.0.1:1313/，监听内容与模板变更
npm run build     # 生产构建与 Pagefind 索引 → public/
npm run check     # 源码引用、站内链接、SEO 与订阅信息检查
```

预览输出到 `public-dev/`，不压缩，也不运行 Pagefind，两种语言均可使用 JSON 搜索。独立使用构建器：

```sh
cargo run --locked --manifest-path ssg/Cargo.toml -- build --source . --dest public --minify
npx -y pagefind@1.3.0 --site public
```

`--minify` 压缩 HTML/CSS，JavaScript 保持原样。构建成功后整体替换目标目录，失败时保留上一份产物；目标目录中不要放手动维护的文件。

## 文件与配置

```text
content/posts/  文章及图片，每篇文章一个目录，index.en.md 为英文版本
content/pages/  关于页等独立页面
content/archives/  归档页文案
ssg/            Rust 构建器与 HTML 模板
assets/         CSS 与 JavaScript
static/         可选的自有静态资源，直接复制到产物
i18n/           实际使用的中英文案
scripts/        源码和静态产物检查
deploy/         可选的 Nginx 配置
site.toml       站名、正式域名、摘要长度和语言配置
```

`site.toml` 的 `title` 是两种语言的站名、署名和订阅作者的统一来源。`base_url` 是正式域名，用于 canonical、SEO、订阅和 sitemap。语言描述按语言单独配置；未知配置项会导致构建失败。

## 静态部署

将 `public/` 的全部内容上传到静态托管平台或普通文件服务器。站点应部署在域名根目录，支持目录默认文件 `index.html`，找不到文件时返回 `404.html` 与 HTTP 404。文章历史别名会生成带 canonical 的静态跳转页。

生产产物可直接本地预览：

```sh
python3 -m http.server 8080 --directory public
```

可选的容器只运行 Nginx 静态服务器，配置见 [`deploy/nginx.conf`](deploy/nginx.conf)，无需环境变量或数据卷：

```sh
podman build -f Containerfile -t personal-blog .
podman run -d --name personal-blog --restart=always \
  -p 127.0.0.1:8787:8787 personal-blog
```

`Containerfile` 的 `artifact` 目标可用于仅导出静态文件。GitHub Actions 执行测试、格式、clippy、完整构建及检查，上传 `static-site` 文件包，通过后发布 `ghcr.io/<owner>/<repository>:latest` 镜像。镜像名来自仓库名。
