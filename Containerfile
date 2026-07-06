# syntax=docker/dockerfile:1

# 单容器：前端（自研 ssg 静态产物 + Pagefind 索引）与后端（Rust + SQLite 留言板）
# 合并进一个 scratch 镜像，由后端进程同时托管静态站和 /api/guestbook/ 接口。

# ---------- 阶段 1：构建静态站点产物（ssg + Pagefind 索引） ----------
# ssg 自身也是 Rust；与后端一样用 alpine/musl，产物是全静态二进制。
# Rust 版本与 rust-toolchain.toml / CI 钉在同一版本，升级时三处一起动。
FROM docker.io/library/rust:1.96.1-alpine AS site

ARG PAGEFIND_VERSION=1.3.0

RUN apk add --no-cache build-base curl

WORKDIR /src
# 先用空壳 crate 编译依赖，利用层缓存：源码变更时不必重编全部依赖。
COPY ssg/Cargo.toml ssg/Cargo.lock ./ssg/
RUN mkdir -p ssg/src \
 && echo 'fn main() {}' > ssg/src/main.rs \
 && cargo build --release --locked --manifest-path ssg/Cargo.toml \
 && rm -rf ssg/src

COPY ssg/src ./ssg/src
# touch 保证 mtime 晚于空壳构建，促使 cargo 重编本 crate（依赖仍走缓存）。
RUN touch ssg/src/main.rs \
 && cargo build --release --locked --manifest-path ssg/Cargo.toml

# Pagefind 发行的 musl 静态二进制，版本与本地保持一致。
RUN curl -fsSL -o /tmp/pagefind.tar.gz \
      "https://github.com/CloudCannon/pagefind/releases/download/v${PAGEFIND_VERSION}/pagefind-v${PAGEFIND_VERSION}-x86_64-unknown-linux-musl.tar.gz" \
 && tar -xzf /tmp/pagefind.tar.gz -C /usr/local/bin \
 && rm -rf /tmp/*

# 站点源码：只拷贝 ssg build 真正会读的路径，backend/ 或文档变更不会
# 让这一层（以及下面的 ssg build + pagefind 索引）缓存失效。
COPY content ./content
COPY assets ./assets
COPY static ./static
COPY i18n ./i18n
COPY site.toml ./site.toml
COPY ssg/templates ./ssg/templates

RUN ./ssg/target/release/ssg build --source . --dest public --minify \
 && pagefind --site public

# ---------- 阶段 2：编译 Rust 后端（bundled SQLite 随 crate 静态编译进 musl 二进制） ----------
# 版本钉法同阶段 1。
FROM docker.io/library/rust:1.96.1-alpine AS build

# bundled SQLite 是 C 代码，需要 musl 头文件与 C 编译器。
RUN apk add --no-cache build-base

WORKDIR /src
# 先用空壳 crate 编译依赖，利用层缓存：源码变更时不必重编全部依赖。
COPY backend/Cargo.toml backend/Cargo.lock ./
RUN mkdir src \
 && echo 'fn main() {}' > src/main.rs \
 && touch src/lib.rs \
 && cargo build --release --locked \
 && rm -rf src

COPY backend/src ./src
# touch 保证 mtime 晚于空壳构建，促使 cargo 重编本 crate（依赖仍走缓存）。
RUN touch src/main.rs src/lib.rs \
 && cargo build --release --locked \
 && mkdir -p /out && cp target/release/backend /out/backend

# 预创建数据目录并赋给非 root 运行用户：scratch 无 shell，运行期无法 chown。
RUN mkdir -p /data && chown 65534:65534 /data

# ---------- 阶段 3：运行镜像（scratch + 单进程） ----------
FROM scratch

COPY --from=build /out/backend /backend
COPY --from=build --chown=65534:65534 /data /data
COPY --from=site  --chown=65534:65534 /src/public /public

# 以 nobody(65534) 身份运行，降低被攻陷后的影响面。
# 注意：bind-mount 到 /data 的宿主目录需在宿主侧 chown 65534:65534；命名卷会继承镜像属主。
USER 65534:65534

# 设置 GUESTBOOK_STATIC_DIR 即进入单容器模式：根路径托管 /public，
# 接口在 /api/guestbook/ 下。容器只监听 HTTP，由 Cloudflare / 反代在前面终止 TLS。
ENV GUESTBOOK_ADDR=0.0.0.0:8787 \
    GUESTBOOK_DB_PATH=/data/guestbook.db \
    GUESTBOOK_STATIC_DIR=/public

EXPOSE 8787
VOLUME ["/data"]

ENTRYPOINT ["/backend"]
