# ssg — suzuka 专用静态站点生成器

为 suzuka-chan.moe **单站定制**的 Rust 生成器，已替代 Hugo 作为生产构建的静态站生成器
（见根目录 `Containerfile` / `package.json`）。不是通用 SSG：只实现本站 `content/` +
`layouts/` 实际用到的功能，其余一律不做。

与 `backend/` 平级、**独立 crate**（不是 workspace 成员），不影响现有后端构建。

Hugo 与 `layouts/`/`hugo.toml` 仍保留在仓库里，作为 `ssg-parity` 对拍的黄金基准来源，
不再参与生产构建。

## 运行

```sh
# 在仓库根目录
cargo run --manifest-path ssg/Cargo.toml -- build --source . --dest public-ssg
```

## 对拍（迁移期的核心手段）

策略：拿现在的 Hugo 产物当**黄金基准**，逐文件 diff，把差异一次性收敛在构建期，
而不是切换上线后才零散发现。

```sh
# 1) 用「不带 --minify」的 Hugo 生成基准（与 ssg 当前输出同为未压缩形态）
hugo --destination golden
# 2) 生成 ssg 产物
cargo run --manifest-path ssg/Cargo.toml -- build --source . --dest public-ssg
# 3) 对拍：逐文件比较，打印首个差异位置
cargo run --manifest-path ssg/Cargo.toml -- diff golden public-ssg
```

CI 里有手动触发的 `ssg-parity` workflow 做同样的事（`.github/workflows/ssg-parity.yml`）。
**只手动触发**，不随 push 跑，作为切换后改动 `content/`/模板时的回归防线。

diff 的归一化规则（有意屏蔽的已知差异，见 `src/diff.rs`）：
- 资源指纹散列 → `HASH`（ssg 暂不做 minify，散列必然不同）
- 无 `src` 的内联 `<script>` 内容 → 占位符（minify 器不同）
- HTML 实体等价形、行尾空白、连续空行

## 当前覆盖范围

| 能力 | 状态 |
|---|---|
| 首页（home.html） | ✅ 首版 |
| 文章页（single.html + 全部 article partials） | ✅ 首版 |
| 双语（zh-cn / en）、i18n 文案、菜单、语言切换 | ✅ 首版 |
| Markdown：typographer、标题自动 ID、图片 figure hook、外链 hook、TOC | ✅ 首版 |
| CJK 词数 / 阅读时长 | ✅ 首版 |
| 别名跳转页（aliases） | ✅ 首版 |
| 资源指纹（兼容 `backend/src/static_site.rs` 的 `.min.<hash>.ext` 判断） | ✅ 命名兼容，⚠️ 暂不真正 minify |
| about / guestbook 独立页（自定义 layout） | ✅ 首版 |
| robots.txt / 404 页 | ✅ 首版 |
| 首页 RSS（feed.xml）/ JSON Feed（feed.json）/ index.json / guestbook-posts.json | ✅ 首版（index.json 的 `content` 截断边界跟 Hugo `strings.Truncate` 有极小尾差，见下） |
| 归档页（archives，双视图 + 分类/标签过滤器 + 自己的 feed.xml） | ✅ 首版，对拍通过 |
| 标签/分类列表页与 term 页（各自的 feed.xml、term 页冗余的 `/p/1/` 跳转桩） | ✅ 首版，对拍通过（列表页 feed.xml 里 term 条目同日期的先后顺序跟黄金基准不完全一致，见下） |
| `/posts/` 隐式 section 列表页（英文有、中文因 `build.render: never` 被关掉，含 redirectTo 的 canonical 覆盖） | ✅ 首版，对拍通过 |
| 分页（`/p/2/` 等） | ✅ 首版——`/en/posts/` 是本站唯一真的会分页的地方（7 篇 > pagerSize 6），其余 term 页条目数都不够，只在 `/p/1/` 出跳转桩 |
| sitemap.xml（根 sitemapindex + 各语言 urlset，含默认语言的 `/zh-cn/` 重定向桩） | ✅ 首版，URL 集合对拍通过（同 lastmod 并列顺序跟黄金基准不完全一致，见下） |
| 代码高亮（syntect → Chroma class） | ⏳ 未实现（本站文章目前无代码块） |
| HTML/CSS/JS minify | ⏳ 未实现（生产此前用 `hugo --minify`；切换后产物比 Hugo 版更大，功能不受影响） |
| Pagefind 搜索索引 | ✅ 独立于生成器，切换后照旧在 `public` 产物上跑 |

**当前状态：产物与 Hugo 黄金基准文件数完全一致（242/242），内容/结构对拍通过。**
仅存的已知差异都是不影响功能的细节（见下面"已知硬骨头"）：HTML 未压缩（体积更大）、
两处同时间戳条目的并列顺序、index.json 一个字段的截断边界。

## 已知硬骨头（诚实记录）

- **og / twitter / schema 三段 meta**：由 `src/build.rs::internal_meta_block` 逐字节手工
  复刻 Hugo 内置模板的空白痕迹，靠 diff harness 逐行对齐收敛，已通过 113/113 对拍。
- **goldmark 排版细节**：typographer 的引号/破折号判定是启发式，长尾差异靠对拍收敛，
  当前 113/113 对拍通过。
- **HTML minify**：生产此前是 `hugo --minify`；切换到 ssg 后**尚未做压缩**，产物体积比
  Hugo 版更大（内容/结构不受影响）。要补上的话，可对 `public/` 产物单独跑一个 minifier，
  与内容生成解耦，不需要改 `build.rs`。
- **对拍工具本身只查 ssg 侧存在的文件**：`ssg diff` 的设计是"迁移期产物是黄金基准的
  子集"，golden 里多出来、ssg 还没实现的文件不算失败——这意味着"对拍全绿"不等于
  "功能完整"，只代表"已经生成的文件都对得上"。判断真实覆盖率要看文件总数
  （`find golden -type f | wc -l` vs `find <产物目录> -type f | wc -l`），不能只看
  `ssg diff` 的退出码。
- **index.json 的 `content` 截断**：复刻 Hugo `strings.Truncate 800`（到达目标长度后，
  往后扫到下一个词边界再切，避免断词，所以结果常比 800 字符更长）本身没问题，但个别
  文章的截断位置跟黄金基准差几十到一百多字符，怀疑是 `.Plain`/`rendered.plain` 在被
  去掉的 `<a>` 标签周围空白处理跟 Hugo 有细微出入，导致第 800 个字符的位置本身就对
  不上。这个字段目前只是一份很少被用到的纯文本预览，没有继续深挖。
- **minijinja 的 `tojson` 会把结果标成"已安全"**：本意是方便塞进 `<script>` 里，但用
  在 HTML 属性值（如 `data-categories`）里就会跳过属性转义，输出裸引号把属性截断。
  `data-categories`/`data-tags` 因此改成 Rust 侧用 `json_attr()`（`serde_json` 序列化后
  走 `escape_attr`）预转义好，模板里 `| safe` 直接插入，不用 `tojson`。
- **Jinja 循环/块标签的空白不会像 Go 模板那样自动裁剪**：Hugo 原模板靠 `{{-`/`-}}`
  精确控制哪里有空行，照抄逻辑但省略这些裁剪标记会在对拍里产生"多一个空行"或
  "缩进多一层"这类纯空白差异（archives 页开发时踩过两次：`{% block main %}` 后要用
  `-%}` 去掉紧跟的空行；分类过滤器循环体的换行要放在 `{% for %}` 标签所在行的末尾，
  不能放在下一行开头，否则每次迭代都会重复插入一次缩进/空行）。
- **URL 的百分号编码路径 ≠ 文件系统路径**：taxonomy term 的 slug 可能是中日文
  （如"视觉小说"），`content::encode_path` 编码后的形式只能用在 href/canonical 这些
  文本里，磁盘上的目录名必须用 `TermAgg.slug` 原始（未编码）的 UTF-8 字符——文章
  slug 因为本来就是纯 ASCII，这个区别之前一直没露出来，加 term 页时才第一次真正踩到。
- **hreflang alternate 的顺序固定按 `config.languages` 的顺序（zh 在前）**，跟"当前
  渲染的是哪个语言"无关；一开始写成"自己排第一、另一个语言排第二"，在渲染英文页时
  顺序就反了——sitemap.xml 和这里的 term/list 页面都要注意。
- **taxonomy 列表页的 feed.xml 和页面上的 term 云用的不是同一个排序**：页面上是
  `site.Taxonomies.*.ByCount`（数量倒序，同数量按标题字母序），但列表页自己的
  `feed.xml` 条目是`.Pages`默认序，实测更接近"这个 term 最新一篇成员文章的日期倒序"，
  同日期的并列顺序没有再深究（跟 sitemap.xml 的并列顺序一样，认为是可接受的已知差异）。
- **归档卡片的封面图逻辑跟 og:image 是两条完全不同的路径**：archive-cover-url.html
  只认 `Params.cover`/`image`/`featured_image`，从来不看 front matter 的 `images`
  （那是 og:image/JSON-LD 用的），永远退到正文里第一张图。
- **redirectTo 的 meta-refresh 块前后各有一个空行**（`</head>` 前的固定空行 + 块自己
  的收尾空行），比表面看起来更"贵"——两侧都要留空行，不能只加一侧，否则后面所有
  内容整体错位一行（`/en/posts/` 这组页面踩过）。
- **pagination-nav 的 for/if 全部要用 `{%-` 左裁剪**：跟 category-filter 循环体一样，
  Hugo 原模板里 `{{- if -}}...{{- else -}}...{{- end }}` 是逐个都裁剪的，模仿时漏一个
  裁剪标记就会在页码之间插入不该有的空行。
- **sitemap.xml 每种语言固定挂在 `/<语言 code>/sitemap.xml`**（如 `/zh-cn/sitemap.xml`），
  不是这门语言真实的 URL 前缀——默认语言真实 URL 没有前缀，但 sitemap 命名空间仍然
  用语言 code，同时 Hugo 还会在这个语言 code 路径下放一个跳回真实首页的重定向桩
  （`/zh-cn/index.html` → `/`）。
- **sitemap `<url>` 块之间没有空行也没有缩进**：第一个 `<url>` 紧跟在 `<urlset>` 开
  标签换行后有 2 格缩进，但后面每个 `<url>` 都是直接跟上一个 `</url>` 贴在一起
  （`</url><url>`），不能对每个条目都用同一套"2 格缩进 + 换行"模板。
