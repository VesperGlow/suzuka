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
| 归档页（archives）、标签/分类列表页与 term 页、`/posts/` 隐式 section 列表页 | ⏳ 未实现（这是目前对拍缺口的大头：archives/categories/tags 相关的 HTML + 各自的 feed.xml、`/en/posts/` 的分页） |
| 分页（`/p/2/` 等） | ⏳ 未实现（只有 `/en/posts/` 这一个 section 因为 7 篇 > pagerSize 6 真的会分页，其余 term 页条目数都不够） |
| 代码高亮（syntect → Chroma class） | ⏳ 未实现（本站文章目前无代码块） |
| HTML/CSS/JS minify | ⏳ 未实现（生产此前用 `hugo --minify`；切换后产物比 Hugo 版更大，功能不受影响） |
| Pagefind 搜索索引 | ✅ 独立于生成器，切换后照旧在 `public` 产物上跑 |

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
- **index.json 的 `content` 截断**：复刻 Hugo `strings.Truncate 800`（截到目标长度后，
  往前扫到下一个词边界再切，避免断词）本身没问题，但个别文章的截断位置跟黄金基准差
  几十到一百多字符，怀疑是 `.Plain`/`rendered.plain` 在被去掉的 `<a>` 标签周围空白
  处理跟 Hugo 有细微出入，导致第 800 个字符的位置本身就对不上。这个字段目前只是一份
  很少被用到的纯文本预览，没有继续深挖。
