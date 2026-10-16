# 静态博客构建器

为「文字庭院」生成静态文件的独立 Rust crate。页面使用 MiniJinja 模板，Markdown 由 pulldown-cmark 渲染；无需框架或运行时服务。

## 使用

在仓库根目录运行：

```sh
cargo run --locked --manifest-path ssg/Cargo.toml -- build --source . --dest public --minify
cargo run --locked --manifest-path ssg/Cargo.toml -- serve --source . --addr 127.0.0.1:1313
```

`build` 默认输出 `public/`；`serve` 默认输出 `public-dev/`，监听输入文件变更并重建。Pagefind 在生成器之外运行；缺少 Pagefind 时搜索使用静态 JSON 索引。

每次构建都在目标目录旁的临时目录中完成，成功后替换旧产物，避免已删除页面和旧指纹资源残留；失败时保留上一份产物。源码及输入目录不可作为输出目录。

## 输入和输出

- `site.toml`：单一站名、正式域名、摘要长度与语言信息；配置结构见 `src/config.rs`，不接受未使用的选项。
- `content/posts/<slug>/index[.en].md`：文章与翻译；图片资源共用一份，构建时生成适用的缩放变体。
- `content/pages/*.md`：独立页面，`layout: about` 使用关于页模板，其余使用普通内容模板。
- `content/archives/_index*.md`：归档页文案；`content/posts/_index*.md` 控制文章列表与分页。
- `i18n/*.toml`：文案及参数插值；日期格式采用 chrono 的 strftime 格式。
- `assets/`：CSS/JS 指纹资源；`static/`：可选的静态资源，目录标记文件不发布。

输出包含双语 HTML、分类与标签页、文章别名跳转页、RSS、JSON Feed、搜索索引、robots.txt 和 sitemap。关于页统计直接写入 HTML；记录时长从已发表文章的最早日期计算，不依赖外部服务。

站名、署名、SEO 与订阅作者统一使用 `site.toml` 的 `title`；所有正式链接使用 `base_url`。RSS 和 JSON Feed 只收录文章。页面没有配图时，不输出图片元数据；文章分享图来自文章自身资源。

## 渲染与维护

- HTML/CSS 可以压缩，JavaScript 原样发布。不要启用内联或外链 JS 压缩：之前使用的压缩器曾改变块作用域，导致浏览器运行时错误。
- URL 路径使用百分号编码，磁盘目录使用原始 UTF-8 名称；非 ASCII 标签必须区分这两种表示。
- `data-categories` / `data-tags` 在 Rust 中序列化并做属性转义，模板只插入已转义值。
- 文章正文的引号排版、标题锚点、词数及摘要截断具有固定规则；修改这些规则时检查现有文章输出和浏览器行为。
- CJK 摘要按字符截断，英文摘要回退到完整词边界；阅读时间按语言的阅读速度估计。
- 默认语言使用根路径，其余语言使用语言前缀；翻译链接仅指向真实存在的对应页面。
- sitemap 分别位于语言目录内，根 sitemap 汇总它们；默认语言目录另有跳回首页的静态页面。

## 验证

```sh
cargo fmt --check --manifest-path ssg/Cargo.toml
cargo clippy --all-targets --locked --manifest-path ssg/Cargo.toml -- -D warnings
cargo test --locked --manifest-path ssg/Cargo.toml
npm run build
npm run check
```

源码检查覆盖模板与资源引用、翻译键；产物检查覆盖站内链接、图片、双语站名、作者元数据、结构化数据、RSS/JSON Feed 和 sitemap。浏览器中还应检查搜索及回退、主题、归档筛选、目录、灯箱和移动端布局。
