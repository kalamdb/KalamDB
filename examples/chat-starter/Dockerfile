# Multi-stage Dockerfile that produces one image runnable as either the
# backend (`server`) or the agent worker (`agent`) — pick the command at
# runtime. The frontend is built statically and served by your CDN or
# Vite preview; this image is for the Node processes only.

# ---- build ----
FROM node:20-alpine AS build
WORKDIR /app

# Install deps from package files first to maximize layer caching.
COPY package.json package-lock.json ./
RUN npm ci

# Then copy the rest and compile.
COPY tsconfig.json tsconfig.agent.json tsconfig.app.json ./
COPY src ./src
COPY server ./server
COPY scripts ./scripts
# Typecheck both app and agent so the image build catches frontend errors
# too — even though the frontend bundle is built separately by your CDN
# step, breaking imports in shared lib code would still ship via the
# tsx-served agent paths.
RUN npx tsc -p tsconfig.agent.json --noEmit && npx tsc -p tsconfig.app.json --noEmit
# tsx runs TS directly at startup; we don't precompile to JS here.

# ---- runtime ----
FROM node:20-alpine AS runtime
ENV NODE_ENV=production
WORKDIR /app

# Production-only npm install — drops devDependencies for a slimmer image.
COPY package.json package-lock.json ./
RUN npm ci --omit=dev

# Source for tsx. If you'd rather ship pre-compiled JS, run `tsc` in the
# build stage and COPY the dist/ output instead.
COPY src ./src
COPY server ./server
COPY scripts ./scripts
COPY tsconfig.json tsconfig.agent.json tsconfig.app.json ./

# Drop privileges.
USER node

# No HEALTHCHECK is set by default — the backend exposes GET /api/health
# which probes the upstream KalamDB. Wire it up in your orchestrator
# (docker-compose healthcheck, k8s readinessProbe, etc.).

# No default CMD — operators MUST override per service:
#   server: ["npx", "tsx", "server/index.ts"]
#   agent:  ["npx", "tsx", "src/agent/index.ts"]
# The hard error below keeps orchestrators from quietly succeeding with no
# process (and recycling the container until quota runs out).
CMD ["node", "-e", "console.error('chat-starter image has no default CMD — set command: [npx, tsx, server/index.ts] or [npx, tsx, src/agent/index.ts] in your compose / k8s spec. See Dockerfile.'); process.exit(1)"]
