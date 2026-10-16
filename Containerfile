# syntax=docker/dockerfile:1

# 构建期运行 Rust 与 Pagefind；运行镜像只用 Nginx 托管静态文件。
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

# 只拷贝构建器读取的站点输入，文档变更不会让这一层缓存失效。
COPY content ./content
COPY assets ./assets
COPY static ./static
COPY i18n ./i18n
COPY site.toml ./site.toml
COPY ssg/templates ./ssg/templates

RUN ./ssg/target/release/ssg build --source . --dest public --minify \
 && pagefind --site public

# 可单独导出产物：podman build --target artifact --output type=local,dest=./dist -f Containerfile .
FROM scratch AS artifact
COPY --from=site /src/public /

# 可选的静态文件服务器；无需数据库、环境变量或数据卷。
FROM docker.io/nginxinc/nginx-unprivileged:stable-alpine AS runtime
COPY deploy/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=site /src/public /usr/share/nginx/html
EXPOSE 8787
