# ssg — suzuka 专用静态站点生成器

为 suzuka-chan.moe **单站定制**的 Rust 生成器，是本站唯一的静态站生成器（见根目录
`Containerfile` / `package.json`）。不是通用 SSG：只实现本站 `content/` 实际用到的
功能，其余一律不做。

与 `backend/` 平级、**独立 crate**（不是 workspace 成员），不影响现有后端构建。

这份生成器最初是逐页对拍 Hugo 产物、照着 Hugo 模板（`layouts/`）手工移植出来的——
迁移验证完成后 Hugo 本身、`layouts/` 和当时用来跑对拍的 `ssg-parity` CI workflow
都已经从仓库里删掉了。仓库根目录的 `hugo.toml` 保留了下来，但现在是 ssg **自己**在读
的配置文件，文件名只是历史遗留，跟 Hugo 已经没有关系。下面"对拍"一节记录的是当时
用来验证迁移正确性的方法论，留着是为了让后面改动 `ssg/` 的人知道这些 `已知硬骨头`
条目是怎么查出来的；里面提到的 `hugo --destination golden` 命令本身已经跑不通了
（没有 `layouts/` 可渲染）。

## 运行

```sh
# 在仓库根目录
cargo run --manifest-path ssg/Cargo.toml -- build --source . --dest public-ssg
# 生产构建加 --minify（压缩 HTML/CSS/JS），package.json / Containerfile 都这么用
cargo run --manifest-path ssg/Cargo.toml -- build --source . --dest public --minify
```

## 对拍（迁移期用过的方法论，Hugo 已经不在仓库里了）

当时的策略：拿 Hugo 产物当**黄金基准**，逐文件 diff，把差异一次性收敛在构建期，
而不是切换上线后才零散发现。命令形式大致是：

```sh
# 1) 用「不带 --minify」的 Hugo 生成基准（与 ssg 当前输出同为未压缩形态）
hugo --destination golden
# 2) 生成 ssg 产物
cargo run --manifest-path ssg/Cargo.toml -- build --source . --dest public-ssg
# 3) 对拍：逐文件比较，打印首个差异位置
cargo run --manifest-path ssg/Cargo.toml -- diff golden public-ssg
```

第 1 步现在跑不了了（`layouts/` 已删除，本地也不再要求装 Hugo）；`diff` 子命令
本身还在（`ssg/src/diff.rs`），如果以后需要比较任意两份产物目录，仍然可以用。
当时 CI 里有个手动触发的 `ssg-parity` workflow 做同样的事，迁移验证完成后已删除。

diff 的归一化规则（有意屏蔽的已知差异，见 `src/diff.rs`）：
- 资源指纹散列 → `HASH`（跟 Hugo 用的不是同一个 minifier，压缩结果逐字节不同，
  散列必然不同）
- 无 `src` 的内联 `<script>` 内容 → 占位符（minify 器不同）
- HTML 实体等价形、行尾空白、连续空行

**对拍故意只比较未压缩产物**：`ssg build`（不加 `--minify`）对应 `hugo`（不加
`--minify`）；两边都不压缩才能逐行对拍出内容/结构级别的差异。`--minify` 的正确性
是另外验证的——不同 minifier 对同一段 HTML/CSS/JS 会做出不同但都合法的压缩选择
（属性间距、实体是否转成字面字符、引号风格……），逐字节对拍两个 minify 之后的产物
没有意义，只会制造大量噪音。验证方法：`--minify` 前后的产物剥掉标签、把已知的
安全实体替换（`&quot;`/`&#x27;`/`&rsquo;`/`&times;` 等）之后转成纯文本比较，
应该逐字节相同——这样能验证压缩没有误删/误改内容，而不要求跟 Hugo 用同一个
minifier。

## 当前覆盖范围

| 能力 | 状态 |
|---|---|
| 首页（home.html） | ✅ 首版 |
| 文章页（single.html + 全部 article partials） | ✅ 首版 |
| 双语（zh-cn / en）、i18n 文案、菜单、语言切换 | ✅ 首版 |
| Markdown：typographer、标题自动 ID、图片 figure hook、外链 hook、TOC | ✅ 首版 |
| CJK 词数 / 阅读时长 | ✅ 首版 |
| 别名跳转页（aliases） | ✅ 首版 |
| 资源指纹（兼容 `backend/src/static_site.rs` 的 `.min.<hash>.ext` 判断） | ✅ 命名兼容，散列基于压缩后内容 |
| about / guestbook 独立页（自定义 layout） | ✅ 首版 |
| robots.txt / 404 页 | ✅ 首版 |
| 首页 RSS（feed.xml）/ JSON Feed（feed.json）/ index.json / guestbook-posts.json | ✅ 首版，对拍通过（含 index.json 的 `content` 截断，见下的 `strings.Truncate` 说明） |
| 归档页（archives，双视图 + 分类/标签过滤器 + 自己的 feed.xml） | ✅ 首版，对拍通过 |
| 标签/分类列表页与 term 页（各自的 feed.xml、term 页冗余的 `/p/1/` 跳转桩） | ✅ 首版，对拍通过（列表页 feed.xml 里 term 条目同日期的先后顺序跟黄金基准不完全一致，见下） |
| `/posts/` 隐式 section 列表页（英文有、中文因 `build.render: never` 被关掉，含 redirectTo 的 canonical 覆盖） | ✅ 首版，对拍通过 |
| 分页（`/p/2/` 等） | ✅ 首版——`/en/posts/` 是本站唯一真的会分页的地方（7 篇 > pagerSize 6），其余 term 页条目数都不够，只在 `/p/1/` 出跳转桩 |
| sitemap.xml（根 sitemapindex + 各语言 urlset，含默认语言的 `/zh-cn/` 重定向桩） | ✅ 首版，URL 集合对拍通过（同 lastmod 并列顺序跟黄金基准不完全一致，见下） |
| 代码高亮（syntect → Chroma class） | ⏳ 未实现（本站文章目前无代码块） |
| HTML/CSS minify（`--minify` 参数，`lightningcss` + `minify-html`） | ✅ 首版，跟 Hugo 用的 minifier 不同，不追求逐字节一致。JS **故意不压缩**，见下 |
| Pagefind 搜索索引 | ✅ 独立于生成器，切换后照旧在 `public` 产物上跑 |

**当前状态：不压缩的产物与 Hugo 黄金基准文件数完全一致（242/242），内容/结构对拍
通过，index.json 的截断也已经逐条核对到位；`--minify` 压缩也已实现并验证内容
无损。** 仅存的已知差异都是不影响功能的细节（见下面"已知硬骨头"）：两处同时间戳
条目的并列顺序、minify 的具体压缩选择跟 Hugo 不同（都合法，只是字节不同）。

## 已知硬骨头（诚实记录）

- **og / twitter / schema 三段 meta**：由 `src/build.rs::internal_meta_block` 逐字节手工
  复刻 Hugo 内置模板的空白痕迹，靠 diff harness 逐行对齐收敛，已通过 113/113 对拍。
- **goldmark 排版细节**：typographer 的引号/破折号判定是启发式，长尾差异靠对拍收敛，
  当前 113/113 对拍通过。
- **JS 故意不压缩——`minify-js` 出过线上事故，不只是会 panic**：一开始用它压缩
  `assets/js/*.js` 和 HTML 里的内联 `<script>`（通过 `minify-html` 的 `minify_js`
  开关）。它不仅对个别合法写法（三元表达式两支返回类型不对称）会内部断言 panic——
  已经用 `catch_unwind` 兜底过——**还会产出不 panic、但真的跑错的代码**：把
  `about-uptime.js` 里的 `updateUptime` 函数声明从它引用的 `if (uptime) { const
  startedAt = ...; ... }` 块里提到了块外面，导致函数体里引用的那些 `const` 变量
  全部拿不到，一调用就是 `ReferenceError`。这个版本合并上线后关于页的运行时长
  计数器直接卡死不动（推断留言板等其他动态功能的异常也是同一批 JS 被错误压缩
  导致的）。修复方式很直接：**完全不让任何工具碰 JS**——`assets.rs` 的 CSS/JS
  处理去掉了 JS 分支，`minify_html_doc` 里 `cfg.minify_js` 固定 `false`，`Cargo.toml`
  也去掉了 `minify-js` 依赖。HTML 空白压缩和 CSS 压缩（`lightningcss`，没出过这类
  正确性问题）继续保留。教训：只做过"剥标签比较文本"级别的完整性校验，没有真的在
  浏览器里跑一遍压缩后的 JS——下次改动到任何压缩管线，必须实际执行一遍脚本逻辑
  再上线，静态检查（哪怕通过 `node --check` 语法检查）不能替代运行时验证。
- **`--minify` 用一个进程内的 `thread_local` 开关，没有到处传参数**：`write_file`/
  `assets::build` 内部读同一个 `Cell<bool>`，`build()` 一开始设一次。构建单进程单次跑，
  没有并发场景，图省事没有把这个参数穿透 17 处 `write_file` 调用点。
- **对拍工具本身只查 ssg 侧存在的文件**：`ssg diff` 的设计是"迁移期产物是黄金基准的
  子集"，golden 里多出来、ssg 还没实现的文件不算失败——这意味着"对拍全绿"不等于
  "功能完整"，只代表"已经生成的文件都对得上"。判断真实覆盖率要看文件总数
  （`find golden -type f | wc -l` vs `find <产物目录> -type f | wc -l`），不能只看
  `ssg diff` 的退出码。
- **index.json 的 `content` 截断——之前的分析是错的，已经用 Hugo 本地实测重新查清楚**：
  最早以为是 `.Plain`/`rendered.plain` 在 `<a>` 标签周围的空白处理跟 Hugo 有出入；
  实际验证下来两边的纯文本**逐字节完全相同**，问题全在 `htmlUnescape` 和
  `strings.Truncate` 本身的行为上，之前都理解错了：
  1. `htmlUnescape` 对所有实体一视同仁地解码，不分排版实体还是 `&quot;`/`&#34;`——
     之前以为 `&quot;`/`&#34;` 解不开，是把 `strings.Truncate` 截断之后**自己重新
     转义引号**（它的返回类型是 `template.HTML`）的产物，误当成了 htmlUnescape
     漏解码。
  2. `strings.Truncate` 本身按脚本类型分两种截断行为，**不是"统一往后扫到下一个
     词边界"**：CJK 硬切在第 N 个字符，不做任何调整；非 CJK（空格分词的语言）
     则是——如果第 N 个字符本身已经是空白就直接切，否则往前（不是往后）找最近的
     空白，丢掉被切开的半个单词。两种脚本的真实行为几乎是镜像对称的，而不是同一
     套逻辑套用在不同文本上恰好表现不同。
  排查方法：往 `content/`/`hugo.toml` 之外单独起一份临时 Hugo 站点副本，加一个自定义
  `PLAIN` output format 把 `.Plain | htmlUnescape | ...` 和 `strings.Truncate 800 ...`
  的中间结果都吐出来，跟 ssg 同一段文本的处理结果逐字符比对——比靠读黄金基准的最终
  产物猜测靠谱得多。现在两种语言全部 14 篇文章的 index.json 条目都逐字节一致。
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
