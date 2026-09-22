# Build from the repository root so the Rust validator and the .NET API share one context:
#   docker build -f server/Dockerfile -t marketplace-api .
#
# Stage 1 builds `validate-source` without the Tauri feature, so no GTK or WebKit.
# Stage 2 builds the Angular portal (website/). Stage 3 publishes the API. Stage 4
# is the runtime: ASP.NET plus Kerberos client libraries for Negotiate on Linux, the
# validator under /app/bin, and the portal and JSON Schemas under /app/wwwroot.

FROM rust:1-bookworm AS validator
WORKDIR /build
COPY src-tauri/Cargo.toml src-tauri/Cargo.lock ./src-tauri/
COPY src-tauri/build.rs src-tauri/windows-app-manifest.xml ./src-tauri/
COPY src-tauri/src ./src-tauri/src
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/src-tauri/target \
    cargo build --manifest-path src-tauri/Cargo.toml --release --no-default-features --bin validate-source \
    && install -m 0755 src-tauri/target/release/validate-source /validate-source

FROM node:22-bookworm-slim AS web
WORKDIR /build
RUN npm install --global pnpm@11.9.0
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY patches ./patches
COPY website/package.json ./website/
RUN --mount=type=cache,target=/root/.local/share/pnpm/store \
    pnpm install --frozen-lockfile --filter website --ignore-scripts
COPY website ./website
RUN pnpm --filter website build

FROM mcr.microsoft.com/dotnet/sdk:10.0 AS build
WORKDIR /src
COPY server/src/Marketplace.Api/Marketplace.Api.csproj server/src/Marketplace.Api/
RUN dotnet restore server/src/Marketplace.Api/Marketplace.Api.csproj
COPY server/src ./server/src
RUN dotnet publish server/src/Marketplace.Api/Marketplace.Api.csproj -c Release -o /app/publish --no-restore

FROM mcr.microsoft.com/dotnet/aspnet:10.0 AS runtime
RUN export DEBIAN_FRONTEND=noninteractive \
    && apt-get update \
    && apt-get install -y --no-install-recommends krb5-user libgssapi-krb5-2 ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=build /app/publish ./
COPY --from=validator /validate-source /app/bin/validate-source
COPY --from=web /build/website/dist/website/browser ./wwwroot
COPY schemas ./wwwroot/schema
ENV ASPNETCORE_URLS=http://0.0.0.0:8080 \
    Validator__Path=/app/bin/validate-source \
    Server__DownloadsPath=/srv/downloads \
    DOTNET_EnableDiagnostics=0
USER app
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/api/health || exit 1
ENTRYPOINT ["dotnet", "Marketplace.Api.dll"]
