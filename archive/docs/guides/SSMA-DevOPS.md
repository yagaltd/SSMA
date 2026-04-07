# SSMA DevOps Guide

Modern deployment strategies for SSMA backend with CSMA frontend, leveraging containers, cloud-native services, and automated CI/CD pipelines.

## Overview

SSMA (backend) + CSMA (frontend) form a complete full-stack application. This guide provides executable deployment strategies:

- **Containerized**: Docker/Podman with orchestration
- **Cloud-Native**: Cloudflare Workers/Pages for frontend + VPS/container for backend  
- **Traditional**: VPS with modern tooling

## Prerequisites

- Node.js 18+ / Bun 1.0+
- Docker or Podman
- SSH access to target server (VPS path)
- Cloudflare account (cloud path)
- Domain name with DNS control

---

## 1. Zero-Downtime VPS Deployment with Docker

### 1.1 Docker Setup

Create `SSMA/Dockerfile`:

```dockerfile
FROM node:20-alpine

WORKDIR /app

# Install dependencies
COPY package*.json ./
RUN npm ci --omit=dev

# Copy application code
COPY . .

# Create non-root user
RUN addgroup -g 1001 ssma && \
    adduser -D -u 1001 -G ssma ssma && \
    chown -R ssma:ssma /app

USER ssma

# Expose port
EXPOSE 5050

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD node -e "require('http').get('http://localhost:' + (process.env.SSMA_PORT || 5050) + '/health', (r) => process.exit(r.statusCode === 200 ? 0 : 1))"

CMD ["node", "src/index.js"]
```

Create `.dockerignore`:

```
node_modules
npm-debug.log
.git
.gitignore
README.md
.env
.env.local
.env.*.local
logs/*.log
tests/
.devcontainer/
.github/
coverage/
.nyc_output/
```

### 1.2 Docker Compose with Traefik

Create `SSMA/docker-compose.yml`:

```yaml
version: '3.8'

services:
  ssma:
    build: .
    container_name: ssma-backend
    restart: unless-stopped
    environment:
      - SSMA_PORT=5050
      - SSMA_JWT_SECRET=${SSMA_JWT_SECRET}
      - SSMA_HMAC_SECRET=${SSMA_HMAC_SECRET}
      - SSMA_ALLOWED_ORIGINS=https://your-domain.com
      - SSMA_LOG_EXPORTER=console
    volumes:
      - ssma-data:/app/data
      - ssma-logs:/app/logs
    networks:
      - traefik
    labels:
      - "traefik.enable=true"
      - "traefik.http.routers.ssma.rule=Host(`api.your-domain.com`)"
      - "traefik.http.routers.ssma.entrypoints=websecure"
      - "traefik.http.routers.ssma.tls.certresolver=letsencrypt"
      - "traefik.http.services.ssma.loadbalancer.server.port=5050"
      - "traefik.http.middlewares.ssma-ratelimit.ratelimit.average=120"
      - "traefik.http.middlewares.ssma-ratelimit.ratelimit.burst=200"
      - "traefik.http.routers.ssma.middlewares=ssma-ratelimit"
    healthcheck:
      test: ["CMD-SHELL", "wget --no-verbose --tries=1 --spider http://localhost:5050/health || exit 1"]
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 40s

  traefik:
    image: traefik:v3.0
    container_name: traefik
    restart: unless-stopped
    command:
      - "--api.dashboard=true"
      - "--api.insecure=false"
      - "--providers.docker=true"
      - "--providers.docker.exposedbydefault=false"
      - "--entrypoints.web.address=:80"
      - "--entrypoints.websecure.address=:443"
      - "--entrypoints.web.http.redirections.entrypoint.to=websecure"
      - "--entrypoints.web.http.redirections.entrypoint.scheme=https"
      - "--certificatesresolvers.letsencrypt.acme.tlschallenge=true"
      - "--certificatesresolvers.letsencrypt.acme.email=admin@your-domain.com"
      - "--certificatesresolvers.letsencrypt.acme.storage=/letsencrypt/acme.json"
      - "--log.level=INFO"
      - "--accesslog=true"
    ports:
      - "80:80"
      - "443:443"
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro
      - traefik-letsencrypt:/letsencrypt
    networks:
      - traefik
    labels:
      - "traefik.enable=true"
      - "traefik.http.routers.traefik.rule=Host(`monitor.your-domain.com`)"
      - "traefik.http.routers.traefik.service=api@internal"
      - "traefik.http.routers.traefik.entrypoints=websecure"
      - "traefik.http.routers.traefik.tls.certresolver=letsencrypt"
      - "traefik.http.middlewares.traefik-auth.basicauth.users=admin:$$apr1$$vR3LkQ9O$$nZ3z6tHq9E8Pj8M9Lk3Kq0"
      - "traefik.http.routers.traefik.middlewares=traefik-auth"

volumes:
  ssma-data:
    driver: local
  ssma-logs:
    driver: local
  traefik-letsencrypt:
    driver: local

networks:
  traefik:
    external: false
```

### 1.3 Deployment Script

Create `SSMA/scripts/deploy/docker-deploy.sh`:

```bash
#!/bin/bash

set -e

# Configuration
VPS_HOST=${VPS_HOST:-"your.server.com"}
VPS_USER=${VPS_USER:-"deploy"}
SSH_KEY=${VPS_SSH_KEY:-"~/.ssh/id_rsa"}
PROJECT_DIR=${VPS_PROJECT_DIR:-"/opt/ssma"}

# Create deployment directory
ssh -i "$SSH_KEY" "$VPS_USER@$VPS_HOST" "mkdir -p $PROJECT_DIR"

# Copy docker files
scp -i "$SSH_KEY" docker-compose.yml Dockerfile .dockerignore "$VPS_USER@$VPS_HOST:$PROJECT_DIR/"

# Copy environment file (create from example if needed)
if [ -f .env ]; then
  scp -i "$SSH_KEY" .env "$VPS_USER@$VPS_HOST:$PROJECT_DIR/.env"
else
  echo "Creating .env from example..."
  scp -i "$SSH_KEY" .env.example "$VPS_USER@$VPS_HOST:$PROJECT_DIR/.env"
fi

# Deploy application code
rsync -avz --exclude-from=.dockerignore \\\n  -e "ssh -i $SSH_KEY" \\\n  . "$VPS_USER@$VPS_HOST:$PROJECT_DIR/app/"

# Build and deploy
ssh -i "$SSH_KEY" "$VPS_USER@$VPS_HOST" << EOF
  cd $PROJECT_DIR
  
  # Pull latest images
  docker compose pull
  
  # Build SSMA image
  docker compose build --no-cache ssma
  
  # Run database migrations if needed
  docker compose run --rm ssma npm run migrate
  
  # Deploy with zero downtime
  docker compose up -d --no-deps ssma
  
  # Wait for health check
  sleep 10
  docker compose ps
  
  # Cleanup old images
  docker image prune -f
EOF

echo "✅ Deployment completed successfully"
echo "SSMA available at: https://api.your-domain.com"
echo "Traefik dashboard at: https://monitor.your-domain.com"
```

Make it executable:

```bash
chmod +x SSMA/scripts/deploy/docker-deploy.sh
```

### 1.4 Alternative: Nginx + Lua

For Nginx with Lua rate limiting, create `nginx-lua.conf`:

```nginx
user nginx;
worker_processes auto;

events {
    worker_connections 1024;
}

http {
    include /etc/nginx/mime.types;
    default_type application/octet-stream;
    
    # Basic settings
    sendfile on;
    tcp_nopush on;
    tcp_nodelay on;
    keepalive_timeout 65;
    types_hash_max_size 2048;
    
    # Rate limiting with lua
    lua_shared_dict rate_limit 10m;
    
    # SSL settings
    ssl_protocols TLSv1.2 TLSv1.3;
    ssl_ciphers ECDHE-RSA-AES256-GCM-SHA512:DHE-RSA-AES256-GCM-SHA512;
    ssl_prefer_server_ciphers off;
    ssl_session_cache shared:SSL:10m;
    
    # Rate limiting function
    lua_package_path "/usr/local/share/lua/5.1/?.lua;;";
    
    server {
        listen 80;
        server_name api.your-domain.com;
        return 301 https://$server_name$request_uri;
    }
    
    server {
        listen 443 ssl http2;
        server_name api.your-domain.com;
        
        ssl_certificate /etc/ssl/certs/your-domain.crt;
        ssl_certificate_key /etc/ssl/private/your-domain.key;
        
        location / {
            access_by_lua_block {
                local limit = require "resty.limit.req"
                local lim, err = limit.new("rate_limit", 120, 0)
                if not lim then
                    ngx.log(ngx.ERR, "failed to instantiate a resty.limit.req object: ", err)
                    return ngx.exit(500)
                end
                
                local key = ngx.var.binary_remote_addr
                local delay, err = lim:incoming(key, true)
                
                if not delay then
                    if err == "rejected" then
                        return ngx.exit(429)
                    end
                    ngx.log(ngx.ERR, "failed to limit req: ", err)
                    return ngx.exit(500)
                end
                
                if delay >= 0.001 then
                    ngx.sleep(delay)
                end
            }
            
            proxy_pass http://localhost:5050;
            proxy_set_header Host $host;
            proxy_set_header X-Real-IP $remote_addr;
            proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
            proxy_set_header X-Forwarded-Proto $scheme;
            
            # WebSocket support
            proxy_http_version 1.1;
            proxy_set_header Upgrade $http_upgrade;
            proxy_set_header Connection "upgrade";
            
            # Timeouts
            proxy_connect_timeout 60s;
            proxy_send_timeout 60s;
            proxy_read_timeout 60s;
        }
        
        location ~* \.(js|css|png|jpg|jpeg|gif|ico|svg)$ {
            expires 1y;
            add_header Cache-Control "public, immutable";
            proxy_pass http://localhost:5050;
        }
    }
}
```

---

## 2. Cloudflare Workers/Pages + Container Backend

### 2.1 Frontend on Cloudflare Pages

Deploy CSMA to Cloudflare Pages:

```bash
cd CSMA
npm run build:prod

# Using wrangler
echo '[\n  name = "csma-frontend"\n  compatibility_date = "2025-01-01"\n  \n  [[pages_build_output_dir]]\n  dir = "dist"\n]' > wrangler.toml

npx wrangler pages deploy dist --project-name=csma-frontend
```

Or via GitHub integration with `cloudflare.yml` workflow:

```yaml
name: Deploy CSMA to Cloudflare Pages

on:
  push:
    branches: [main]

jobs:
  deploy:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      
      - uses: actions/setup-node@v3
        with:
          node-version: 20
          cache: 'npm'
      
      - run: npm ci
      
      - run: npm run build:prod
        env:
          VITE_API_URL: "https://api.your-domain.com"
      
      - name: Deploy to Cloudflare Pages
        uses: cloudflare/pages-action@v1
        with:
          apiToken: ${{ secrets.CLOUDFLARE_API_TOKEN }}
          accountId: ${{ secrets.CLOUDFLARE_ACCOUNT_ID }}
          projectName: csma-frontend
          directory: dist
          gitHubToken: ${{ secrets.GITHUB_TOKEN }}
```

### 2.2 Backend Container Hosting Options

#### Option A: Fly.io (Edge Containers)

```bash
# Install flyctl
curl -L https://fly.io/install.sh | sh

# Setup
cd SSMA
flyctl launch --name ssma-backend --region lax --port 5050

# Update fly.toml
cat > fly.toml << EOF
app = "ssma-backend"
primary_region = "lax"

[build]
  dockerfile = "Dockerfile"

[env]
  SSMA_PORT = "5050"
  SSMA_ALLOWED_ORIGINS = "https://your-domain.pages.dev,https://csma-frontend.pages.dev"
  SSMA_LOG_EXPORTER = "console"

[http_service]
  internal_port = 5050
  force_https = true
  auto_stop_machines = true
  auto_start_machines = true
  min_machines_running = 1
  processes = ["app"]

  [http_service.http_options.response.headers]
    X-Frame-Options = "DENY"
    X-Content-Type-Options = "nosniff"
    Referrer-Policy = "strict-origin-when-cross-origin"

[[vm]]
  cpu_kind = "shared"
  cpus = 1
  memory_mb = 512
EOF

# Deploy
flyctl deploy --ha --strategy bluegreen
```

#### Option B: Railway/Heroku Alternative

```dockerfile
# Add to Dockerfile
FROM node:20-alpine AS builder
WORKDIR /app
COPY package*.json ./
RUN npm ci --omit=dev
COPY . .

FROM node:20-alpine AS runner
WORKDIR /app
COPY --from=builder /app /app
RUN addgroup -g 1001 ssma && adduser -D -u 1001 -G ssma ssma
USER ssma
EXPOSE $PORT
CMD ["node", "src/index.js"]
```

```bash
# Railway auto-detects Dockerfile
railway login
railway link
railway variables set SSMA_JWT_SECRET="..."
railway up
```

### 2.3 WebSocket Support on Cloudflare

For WebSocket support through Cloudflare:

```javascript
// In SSMA, upgrade WebSocket connections
import { WebSocketServer } from 'ws';

const wss = new WebSocketServer({ server });

wss.on('connection', (ws, req) => {
  // Verify auth token
  const token = new URL(req.url, 'http://localhost').searchParams.get('token');
  
  ws.on('message', (data) => {
    // Handle real-time events
    broadcastToChannel(ws, data);
  });
});
```

Cloudflare automatically proxies WebSocket connections on paid plans.

---

## 3. GitHub Actions CI/CD Pipeline

### 3.1 Full Stack Deployment

Create `.github/workflows/deploy-fullstack.yml`:

```yaml
name: Deploy SSMA + CSMA Full Stack

on:
  push:
    branches: [main]
  workflow_dispatch:

env:
  REGISTRY: ghcr.io
  IMAGE_NAME: ${{ github.repository }}/ssma

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Setup Node.js
        uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: 'npm'
      
      - name: Install dependencies
        run: |
          cd SSMA && npm ci
          cd ../CSMA && npm ci
      
      - name: Run SSMA tests
        run: cd SSMA && npm test
      
      - name: Run CSMA tests
        run: cd CSMA && npm run test:contracts && npm run test:validation
      
      - name: Synchronize contracts
        run: cd SSMA && npm run sync:contracts
  
  build-and-push:
    needs: test
    runs-on: ubuntu-latest
    permissions:
      contents: read
      packages: write
    steps:
      - uses: actions/checkout@v4
      
      - name: Log in to Container Registry
        uses: docker/login-action@v3
        with:
          registry: ${{ env.REGISTRY }}
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}
      
      - name: Extract metadata
        id: meta
        uses: docker/metadata-action@v5
        with:
          images: ${{ env.REGISTRY }}/${{ env.IMAGE_NAME }}
          tags: |
            type=ref,event=branch
            type=sha,prefix=sha-
            type=raw,value=latest,enable={{is_default_branch}}
      
      - name: Build and push SSMA image
        uses: docker/build-push-action@v5
        with:
          context: ./SSMA
          push: true
          tags: ${{ steps.meta.outputs.tags }}
          labels: ${{ steps.meta.outputs.labels }}
  
  deploy-frontend:
    needs: build-and-push
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Setup Node.js
        uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: 'npm'
      
      - name: Install dependencies
        run: cd CSMA && npm ci
      
      - name: Build frontend
        run: |
          cd CSMA
          npm run build:prod
        env:
          VITE_API_URL: "https://api.your-domain.com"
      
      - name: Deploy to Cloudflare Pages
        uses: cloudflare/pages-action@v1
        with:
          apiToken: ${{ secrets.CLOUDFLARE_API_TOKEN }}
          accountId: ${{ secrets.CLOUDFLARE_ACCOUNT_ID }}
          projectName: csma-frontend
          directory: CSMA/dist
          gitHubToken: ${{ secrets.GITHUB_TOKEN }}
  
  deploy-backend:
    needs: build-and-push
    runs-on: ubuntu-latest
    environment: production
    steps:
      - name: Deploy to VPS
        uses: appleboy/ssh-action@v1.0.0
        with:
          host: ${{ secrets.VPS_HOST }}
          username: ${{ secrets.VPS_USER }}
          key: ${{ secrets.VPS_SSH_KEY }}
          script: |
            cd /opt/ssma
            
            # Login to GitHub Container Registry
            echo ${{ secrets.GITHUB_TOKEN }} | docker login ghcr.io -u ${{ github.actor }} --password-stdin
            
            # Pull new image
            docker compose pull ssma
            
            # Rolling update
            docker compose up -d --no-deps ssma
            
            # Wait for health check
            sleep 15
            docker compose ps
            
            # Health check
            if ! docker compose exec -T ssma wget --no-verbose --tries=3 --spider http://localhost:5050/health; then
              echo "Health check failed, rolling back..."
              docker compose rollback ssma
              exit 1
            fi
            
            # Cleanup
            docker image prune -f
            docker logout ghcr.io
```

---

## 4. Environment Configuration

### 4.1 Production Environment Variables

Create `SSMA/.env.production`:

```bash
# Server
SSMA_PORT=5050

# Security
SSMA_JWT_SECRET=<generate-with: openssl rand -hex 32>
SSMA_JWT_ISSUER=ssma-auth-service
SSMA_JWT_AUDIENCE=csma-kit
SSMA_ACCESS_TTL_MS=900000
SSMA_REFRESH_TTL_MS=604800000
SSMA_HMAC_SECRET=<generate-with: openssl rand -hex 32>
SSMA_HMAC_TTL_MS=300000
SSMA_ALLOWED_ORIGINS=https://your-domain.com,https://csma-frontend.pages.dev

# Rate limiting
SSMA_RATE_WINDOW_MS=60000
SSMA_RATE_MAX=120

# Logging
SSMA_LOG_EXPORTER=console,file
SSMA_LOG_BUFFER_SIZE=2000
SSMA_LOG_FILE=logs/ssma.log
SSMA_LOG_MAX_BATCH=200

# Features
SSMA_STATIC_RENDER_ENABLED=true
SSMA_MONITOR_BACKLOG_THRESHOLD=1000
SSMA_MONITOR_INVALIDATION_BUDGET_MS=5000
```

Generate secrets:

```bash
# Generate JWT and HMAC secrets
openssl rand -hex 32
openssl rand -hex 32
```

### 4.2 Local Development with Docker

```bash
# Local development
docker compose -f docker-compose.dev.yml up

# Watch mode with nodemon (Docker)
docker compose -f docker-compose.dev.yml --profile dev up
```

Create `docker-compose.dev.yml`:

```yaml
version: '3.8'

services:
  ssma:
    build: .
    environment:
      - SSMA_PORT=5050
      - SSMA_ALLOWED_ORIGINS=http://localhost:5173,http://localhost:4173
      - SSMA_LOG_EXPORTER=console
    ports:
      - "5050:5050"
    volumes:
      - .:/app
      - /app/node_modules
    command: npm run dev
```

---

## 5. Monitoring and Observability

### 5.1 Structured Logging

SSMA already includes LogAccumulator. Configure exporters:

```bash
# Console + File (default)
SSMA_LOG_EXPORTER=console,file

# Add HTTP export for external service
SSMA_LOG_EXPORTER=console,file,http
SSMA_LOG_HTTP_ENDPOINT=https://logs.your-domain.com/batch
SSMA_LOG_HTTP_TOKEN=<your-token>
```

### 5.2 Health Checks

SSMA exposes health endpoints:

```bash
# Service health
curl https://api.your-domain.com/health

# Log pipeline health
curl https://api.your-domain.com/logs/health

# Metrics (if enabled)
curl https://api.your-domain.com/metrics
```

### 5.3 Uptime Monitoring

Add to `docker-compose.yml`:

```yaml
  uptime-kuma:
    image: louislam/uptime-kuma:latest
    container_name: uptime-kuma
    restart: unless-stopped
    ports:
      - "3001:3001"
    volumes:
      - uptime-kuma:/app/data
    networks:
      - traefik
    labels:
      - "traefik.enable=true"
      - "traefik.http.routers.uptime-kuma.rule=Host(`status.your-domain.com`)"
      - "traefik.http.routers.uptime-kuma.entrypoints=websecure"
      - "traefik.http.routers.uptime-kuma.tls.certresolver=letsencrypt"
```

---

## 6. Backup and Disaster Recovery

### 6.1 Automated Backups

Add to docker-compose.yml:

```yaml
  backup:
    image: offen/docker-volume-backup:latest
    restart: unless-stopped
    environment:
      - BACKUP_FILENAME=ssma-backup-%Y-%m-%dT%H-%M-%S.tar.gz
      - BACKUP_PRUNING_PREFIX=ssma-backup-
      - BACKUP_RETENTION_DAYS=7
      - AWS_ACCESS_KEY_ID=${BACKUP_AWS_KEY}
      - AWS_SECRET_ACCESS_KEY=${BACKUP_AWS_SECRET}
      - AWS_ENDPOINT_URL=${BACKUP_ENDPOINT:-https://s3.amazonaws.com}
      - BACKUP_CRON_EXPRESSION=0 2 * * *
    volumes:
      - ssma-data:/backup/ssma-data:ro
      - ssma-logs:/backup/ssma-logs:ro
      - /var/run/docker.sock:/var/run/docker.sock:ro
```

### 6.2 Manual Backup Script

Create `SSMA/scripts/backup.sh`:

```bash
#!/bin/bash

set -e

BACKUP_DIR="/opt/backups/ssma"
DATE=$(date +%Y%m%d_%H%M%S)
BUCKET="s3://your-backup-bucket/ssma"

# Create backup directory
mkdir -p "$BACKUP_DIR"

# Backup data volume
docker run --rm \
  -v ssma-data:/data \
  -v "$BACKUP_DIR":/backup \
  alpine tar czf /backup/ssma-data-$DATE.tar.gz /data

# Upload to S3
aws s3 cp "$BACKUP_DIR/ssma-data-$DATE.tar.gz" "$BUCKET/"

# Cleanup old backups (keep 7 days)
find "$BACKUP_DIR" -name "ssma-*.tar.gz" -mtime +7 -delete
aws s3 ls "$BUCKET/" | awk '{print $4}' | while read file; do
  # Cleanup S3 backups older than 7 days
  echo "Managing backup: $file"
done

echo "✅ Backup completed: ssma-data-$DATE.tar.gz"
```

---

## 7. Migration from Legacy Deployment

### 7.1 Migration Checklist

```bash
# 1. Export current data
cd SSMA
cp -r data data-backup-$(date +%Y%m%d)

# 2. Verify environment variables
diff .env .env.production

# 3. Test Docker build
docker build -t ssma:test .
docker run --rm -it --env-file .env.production -p 5050:5050 ssma:test

# 4. Deploy Traefik first
docker compose up -d traefik

# 5. Deploy SSMA
docker compose up -d ssma

# 6. Update DNS
# Point api.your-domain.com to new server

# 7. Verify with health checks
curl -f https://api.your-domain.com/health

# 8. Decommission old deployment
# PM2 stop ssma
# systemctl stop ssma
```

### 7.2 Rollback Plan

```bash
# If issues occur, rollback immediately
docker compose down
docker compose up -d ssma:previous-version

# Or revert to PM2
npm start
```

---

## 8. Quick Start Commands

```bash
# Production deployment (VPS)
cd SSMA
./scripts/deploy/docker-deploy.sh

# Cloud deployment
cd CSMA
npm run deploy:cdn  # Frontend to Cloudflare
flyctl deploy       # Backend to Fly.io

# Local development
docker compose -f docker-compose.dev.yml up

# Health check
curl https://api.your-domain.com/health
```

---

## Security Considerations

- ✅ Secrets managed via environment variables, never committed
- ✅ Automatic SSL with Traefik + Let's Encrypt
- ✅ Rate limiting at reverse proxy level
- ✅ Non-root Docker containers
- ✅ Health checks and auto-restart
- ✅ Network isolation with Docker networks
- ✅ Read-only filesystem where possible
- ✅ Regular security updates via `docker compose pull`

---

## Cost Optimization

- **Development**: Docker Compose locally
- **Staging**: Single Fly.io machine (free tier)
- **Production**: 
  - Frontend: Cloudflare Pages (free)
  - Backend: $5-10/month VPS or Fly.io
  - Total: $10-20/month vs $50+ for traditional VPS setup

---

## Troubleshooting

### Common Issues

```bash
# View logs
docker compose logs -f ssma

# Check container status
docker compose ps

# Restart service
docker compose restart ssma

# SSH into container
docker compose exec ssma sh

# Reset database (dev only)
docker compose down -v
docker compose up -d

# Clear Traefik cache
docker compose exec traefik rm -rf /letsencrypt/acme.json
```

### Performance Tuning

```bash
# Monitor resources
docker stats

# Scale horizontally (if needed)
docker compose up -d --scale ssma=3

# Tune Node.js memory
environment:
  - NODE_OPTIONS=--max-old-space-size=4096
```

---

## 9. Cloudflare Durable Objects WebSocket Architecture (Advanced)

For production deployments with high WebSocket concurrency (1000+ concurrent connections), global users, or strict security requirements, this architecture moves WebSocket handling to Cloudflare's edge while keeping your SSMA backend completely private.

### 9.1 Architecture Overview

```
CSMA Frontend (Cloudflare Pages)
    ↓ wss://websocket.your-domain.com
Cloudflare Durable Objects (WebSocket Hub)
    ↓ HTTP/WebSocket (via Cloudflare Tunnel)
Your Private SSMA Server (localhost:5050)
```

**How it works:**
- Frontend connects directly to Durable Object at edge (closest to user)
- Durable Object maintains persistent WebSocket connections with clients
- Durable Object forwards messages to your origin via Cloudflare Tunnel
- Your SSMA server stays completely private - never exposed to internet
- No nginx/traefik needed for WebSocket traffic

### 9.2 Benefits

**Security:**
- Zero origin exposure - server has no public IP or open ports
- DDoS protection absorbed by Cloudflare before reaching infrastructure
- Authentication validation at edge (JWT verification in Durable Object)
- Automatic TLS end-to-end

**Performance:**
- Connections terminate closest to users (latency: 50ms → 5-10ms)
- Durable Objects handle connection management and buffering
- Reduced origin bandwidth and CPU for connection overhead
- Global distribution without infrastructure complexity

**Operational:**
- No reverse proxy configuration needed for WebSocket
- Automatic scaling per user/channel
- Built-in rate limiting at edge
- Connection state persists through origin restarts

### 9.3 Implementation

Create `workers/websocket-hub.js`:

```javascript
export class WebSocketHub {
  constructor(state, env) {
    this.sessions = new Map(); // clientId -> WebSocket
    this.originWS = null;
    this.state = state;
    this.env = env;
    this.originUrl = env.ORIGIN_WS_URL || 'wss://ssma-internal.your-tunnel.com/optimistic/ws';
  }

  async fetch(request) {
    const url = new URL(request.url);
    
    // Handle WebSocket upgrade from client
    if (request.headers.get('Upgrade') === 'websocket') {
      return this.handleClientConnection(request);
    }
    
    // Health check for Durable Object
    if (url.pathname === '/health') {
      return new Response(JSON.stringify({
        connections: this.sessions.size,
        originConnected: this.originWS?.readyState === WebSocket.OPEN
      }), { 
        headers: { 'Content-Type': 'application/json' }
      });
    }
    
    return new Response('Not Found', { status: 404 });
  }

  async handleClientConnection(request) {
    // Authenticate at edge (optional but recommended)
    const token = this.extractToken(request);
    if (!token) {
      return new Response('Unauthorized', { status: 401 });
    }
    
    // Verify token with auth service
    const user = await this.verifyToken(token);
    if (!user) {
      return new Response('Forbidden', { status: 403 });
    }
    
    // Create WebSocket pair
    const pair = new WebSocketPair();
    const [client, server] = pair;

    // Accept the client's WebSocket
    server.accept();

    // Connect to origin if not already connected
    if (!this.originWS || this.originWS.readyState !== WebSocket.OPEN) {
      await this.connectToOrigin();
    }

    // Generate client ID
    const clientId = crypto.randomUUID();
    this.sessions.set(clientId, server);

    // Handle messages from client
    server.addEventListener('message', async (event) => {
      try {
        // Forward to origin
        if (this.originWS?.readyState === WebSocket.OPEN) {
          const message = JSON.parse(event.data);
          message.clientId = clientId; // Tag with client ID
          message.user = user; // Include auth context
          this.originWS.send(JSON.stringify(message));
        }
      } catch (error) {
        console.error('[WS Hub] Failed to forward message:', error);
        server.send(JSON.stringify({ 
          type: 'error', 
          code: 'RELAY_ERROR',
          message: 'Failed to relay message to origin'
        }));
      }
    });

    // Handle client disconnect
    server.addEventListener('close', () => {
      this.sessions.delete(clientId);
      
      // Notify origin of disconnect
      if (this.originWS?.readyState === WebSocket.OPEN) {
        this.originWS.send(JSON.stringify({
          type: 'client.disconnect',
          clientId: clientId
        }));
      }
    });

    // Return the client WebSocket
    return new Response(null, { 
      status: 101, 
      webSocket: client 
    });
  }

  async connectToOrigin() {
    try {
      this.originWS = new WebSocket(this.originUrl);
      
      this.originWS.addEventListener('open', () => {
        console.log('[WS Hub] Connected to origin');
      });

      this.originWS.addEventListener('message', (event) => {
        try {
          const message = JSON.parse(event.data);
          
          // Broadcast to all clients or specific client
          if (message.clientId && this.sessions.has(message.clientId)) {
            // Send to specific client
            this.sessions.get(message.clientId).send(event.data);
          } else {
            // Broadcast to all clients
            for (const [id, ws] of this.sessions) {
              if (ws.readyState === WebSocket.OPEN) {
                ws.send(event.data);
              }
            }
          }
        } catch (error) {
          console.error('[WS Hub] Failed to broadcast:', error);
        }
      });

      this.originWS.addEventListener('close', () => {
        console.warn('[WS Hub] Origin connection closed');
        // Attempt to reconnect
        setTimeout(() => this.connectToOrigin(), 5000);
      });

      this.originWS.addEventListener('error', (error) => {
        console.error('[WS Hub] Origin WebSocket error:', error);
      });

    } catch (error) {
      console.error('[WS Hub] Failed to connect to origin:', error);
      setTimeout(() => this.connectToOrigin(), 5000);
    }
  }

  extractToken(request) {
    // Extract from Authorization header or cookie
    const authHeader = request.headers.get('Authorization');
    if (authHeader?.startsWith('Bearer ')) {
      return authHeader.slice(7);
    }
    
    const cookie = request.headers.get('Cookie');
    if (cookie) {
      const match = cookie.match(/ssma_session=([^;]+)/);
      return match ? match[1] : null;
    }
    
    return null;
  }

  async verifyToken(token) {
    // Call auth service or verify JWT
    // For faster validation, consider storing public keys in env
    try {
      const response = await fetch('https://api.your-domain.com/auth/verify', {
        headers: { 'Authorization': `Bearer ${token}` }
      });
      
      if (!response.ok) return null;
      return await response.json();
    } catch (error) {
      console.error('[WS Hub] Token verification failed:', error);
      return null;
    }
  }
}

// Worker entry point
export default {
  async fetch(request, env, ctx) {
    const url = new URL(request.url);
    
    // Route WebSocket connections to Durable Object
    if (url.pathname === '/optimistic/ws') {
      const id = env.WEBSOCKET_HUB.get(env.WEBSOCKET_HUB.newUniqueId());
      return id.fetch(request);
    }
    
    // All other requests go to origin via tunnel
    return fetch(request);
  }
}
```

Create `workers/wrangler.toml`:

```toml
name = "ssma-websocket-hub"
main = "websocket-hub.js"
compatibility_date = "2025-01-01"
compatibility_flags = ["nodejs_compat"]

[env.production]
# Stored in Cloudflare dashboard or wrangler secret
ORIGIN_WS_URL = "wss://ssma-internal.your-tunnel.com/optimistic/ws"

[[durable_objects.bindings]]
name = "WEBSOCKET_HUB"
class_name = "WebSocketHub"

[[migrations]]
tag = "v1"
new_classes = ["WebSocketHub"]
```

### 9.4 Cloudflare Tunnel Setup

On your private server:

```bash
curl -L --output cloudflared.deb https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64.deb
sudo dpkg -i cloudflared.deb

# Authenticate
cloudflared login

# Create tunnel
cloudflared tunnel create ssma-origin

# Configure tunnel
mkdir -p ~/.cloudflared
cat > ~/.cloudflared/config.yml << EOF
tunnel: ssma-origin
credentials-file: /home/$USER/.cloudflared/$(cloudflared tunnel list | grep ssma-origin | awk '{print $1}').json

ingress:
  - hostname: ssma-internal.your-tunnel.com
    service: http://localhost:5050
    originRequest:
      httpHostHeader: ssma-internal.your-tunnel.com
      noTLSVerify: false
  
  - service: http_status:404
EOF

# Install as systemd service
sudo cloudflared service install
sudo systemctl enable cloudflared
sudo systemctl start cloudflared

# Create DNS record
cloudflared tunnel route dns ssma-origin ssma-internal.your-tunnel.com
```

### 9.5 Deploy WebSocket Hub

```bash
# Deploy to Cloudflare Workers
cd workers
npx wrangler deploy --env production

# Set secrets
npx wrangler secret put ORIGIN_WS_URL --env production
# Enter: wss://ssma-internal.your-tunnel.com/optimistic/ws
```

### 9.6 Update Frontend Configuration

In CSMA `config.js`:

```javascript
export const config = {
  API_URL: 'https://api.your-domain.com', // HTTP API via tunnel
  WS_URL: 'wss://websocket.your-domain.com/optimistic/ws', // WebSocket via Durable Object
  
  // For local development
  // WS_URL: 'ws://localhost:5050/optimistic/ws'
};
```

### 9.7 Route Configuration

In Cloudflare dashboard:

1. **WebSocket route** (must be first):
   - Route: `websocket.your-domain.com/optimistic/ws*`
   - Worker: `ssma-websocket-hub`

2. **API routes** (all other traffic):
   - Route: `api.your-domain.com/*`
   - Origin: `ssma-internal.your-tunnel.com` (via tunnel)

3. **Frontend**:
   - Route: `your-domain.com/*`
   - Pages: `csma-frontend`

### 9.8 Monitoring and Debugging

Check Durable Object metrics:

```bash
# Real-time logs
npx wrangler tail --env production

# View Durable Object instances
npx wrangler durable-object namespace get WEBSOCKET_HUB --env production
```

Monitor connections:

```bash
curl https://websocket.your-domain.com/health
```

Expected response:

```json
{
  "connections": 127,
  "originConnected": true
}
```

### 9.9 Cost Optimization

**Durable Objects pricing:**
- $5/month minimum + $0.15/GB data transfer
- Each Durable Object handles ~1000 concurrent connections
- For < 1000 connections: use single DO per application
- For > 1000 connections: shard by userId or channel

**Example sharding:**

```javascript
// In worker fetch handler:
const userId = extractUserId(request);
const doId = env.WEBSOCKET_HUB.idFromName(userId); // Consistent hashing
const stub = env.WEBSOCKET_HUB.get(doId);
return stub.fetch(request);
```

**Bandwidth savings:**
- 70-90% reduction in origin bandwidth (Cloudflare handles keepalives)
- 50-80% reduction in origin CPU (connection management offloaded)

### 9.10 Comparison: Traditional vs Durable Objects

| Feature | Traditional (nginx/traefik) | Durable Objects |
|---------|---------------------------|-----------------|
| **Origin exposure** | Public IP + open ports | Zero exposure |
| **DDoS protection** | Manual setup | Built-in |
| **Global latency** | 50-200ms | 5-20ms |
| **Connection limit** | ~10-50k per server | 100k+ per DO |
| **Infrastructure** | nginx/traefik required | None |
| **Auto-scaling** | Manual | Automatic |
| **Setup complexity** | Medium-High | Medium |
| **Vendor lock-in** | Low | High |
| **Cost (1000+ WS)** | $20-50/month | $5-15/month |

**When to use:**
- ✅ 1000+ concurrent WebSocket connections
- ✅ Global user base
- ✅ Security-sensitive applications
- ✅ Small teams wanting infrastructure simplicity
- ✅ Real-time features (chat, multiplayer, live updates)

**When to avoid:**
- ❤️ ️Ultra-low latency trading (<5ms requirement)
- ❤️ ️Extreme vendor lock-in concerns
- ❤️ ️Complex binary WebSocket protocols
- ❤️ ️< 100 concurrent connections (overkill)

---

## 10. Cloudflare CDN + Tunnel (Simple WebSocket Passthrough)

For applications with moderate WebSocket concurrency (<500 connections), this minimalist approach connects your frontend directly to your SSMA backend via Cloudflare Tunnel - no reverse proxy, load balancer, or code changes required.

### 10.1 Architecture Overview

```
CSMA Frontend (Cloudflare Pages)
    ↓ wss://api.your-domain.com/optimistic/ws
Cloudflare Tunnel (cloudflared)
    ↓ forwards to
Your SSMA Server (localhost:5050)
```

**How it works:**
- Frontend connects to `wss://api.your-domain.com/optimistic/ws`
- Cloudflare routes through tunnel to your private server
- Express/WebSocket server handles upgrades normally
- Zero origin exposure, no reverse proxy needed

### 10.2 Benefits

**Simplicity:**
- Zero code changes to existing WebSocket handlers
- No reverse proxy (nginx/traefik) configuration
- Works with existing SyncGateway implementation
- Tunnel handles SSL termination automatically

**Security:**
- Server has no public IP or open ports
- DDoS protection before traffic reaches origin
- Automatic TLS (Cloudflare → Tunnel → Origin)
- Same origin protection as Durable Objects

**Operational:**
- Single tunnel for all traffic (HTTP + WebSocket)
- No infrastructure to manage beyond cloudflared
- Works with Docker or bare metal deployment
- Integrated into existing CI/CD pipeline

### 10.3 Setup

**On your SSMA server:**

```bash
# Install cloudflared
curl -L --output cloudflared.deb https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64.deb
sudo dpkg -i cloudflared.deb

# Authenticate (opens browser)
cloudflared login

# Create tunnel
cloudflared tunnel create ssma-api

# Configure tunnel
mkdir -p ~/.cloudflared
cat > ~/.cloudflared/config.yml << EOF
tunnel: ssma-api
credentials-file: /home/$USER/.cloudflared/$(cloudflared tunnel list | grep ssma-api | awk '{print $1}').json

ingress:
  - hostname: api.your-domain.com
    service: http://localhost:5050
    originRequest:
      httpHostHeader: api.your-domain.com
      noTLSVerify: false
  
  - service: http_status:404
EOF

# Create DNS record
cloudflared tunnel route dns ssma-api api.your-domain.com

# Install as service
sudo cloudflared service install
sudo systemctl enable cloudflared
sudo systemctl start cloudflared
```

**Verify tunnel:**

```bash
# Check status
sudo systemctl status cloudflared

# Test connection
curl https://api.your-domain.com/health

# WebSocket test
wscat -c wss://api.your-domain.com/optimistic/ws
```

### 10.4 Frontend Configuration

Update CSMA `config.js`:

```javascript
export const config = {
  API_URL: 'https://api.your-domain.com',
  WS_URL: 'wss://api.your-domain.com/optimistic/ws', // Same domain!
  
  // Development
  // WS_URL: 'ws://localhost:5050/optimistic/ws'
};
```

### 10.5 Cloudflare Settings

In Cloudflare dashboard:

1. **WebSockets (disable proxy mode):**
   - Go to **DNS** settings
   - Find `api.your-domain.com` record
   - Set **Proxy status** to **DNS only** (gray cloud)
   - ☝️ Important: Orange cloud (proxied) breaks tunnel WebSockets

2. **Or keep orange cloud and configure:**
   - Go to **Network** settings
   - Enable **WebSockets**
   - Set **Maximum upload size** to 100MB

### 10.6 Monitoring

```bash
# Tunnel metrics
curl http://localhost:41279/metrics

# Connection status
cloudflared tunnel info ssma-api

# Real-time logs
journalctl -u cloudflared -f
```

### 10.7 Comparison: Direct Tunnel vs Durable Objects

| Feature | Direct Tunnel | Durable Objects |
|---------|---------------|-----------------|
| **Server changes** | ❌ None required | ✅ Worker code needed |
| **Setup time** | 5 minutes | 30-60 minutes |
| **Origin connections** | 1:1 per client | 1:1000+ multiplexed |
| **Connection pooling** | ❌ No | ✅ Yes |
| **Edge processing** | ❌ No | ✅ Auth, validation |
| **Bandwidth savings** | 0-20% | 70-90% |
| **Global latency** | 30-100ms | 5-20ms |
| **Max connections** | ~10k per server | 100k+ per object |
| **Best for** | <500 concurrent WS | 1000+ concurrent WS |

### 10.8 Cost

**Free tier:**
- Cloudflare Tunnel: $0 (unlimited bandwidth)
- Cloudflare CDN/Pages: $0
- Bandwidth: $0 (Cloudflare egress is free)

**Paid plans:**
- None required for most applications
- Scale to 10k+ connections without cost increase

### 10.9 When to Use

**✅ Perfect for:**
- MVP and prototyping
- Internal tools (<100 users)
- Moderate real-time features
- Teams new to DevOps
- When simplicity > scale

**⚠️ Consider Durable Objects instead when:**
- 1000+ concurrent WebSocket connections
- Global user base requiring edge optimization
- High connection churn (>100 connects/sec)
- Need edge authentication/processing

### 10.10 Production Checklist

- [ ] Tunnel is running: `systemctl is-active cloudflared`
- [ ] Firewall blocks port 5050 externally
- [ ] HTTPS works: `curl -f https://api.your-domain.com/health`
- [ ] WebSocket connects: `wscat -c wss://...`
- [ ] Frontend configured with production URLs
- [ ] Monitoring in place (uptime checks)
- [ ] Logs shipping to centralized service
- [ ] Backup strategy configured

---

## Conclusion

This DevOps guide provides:

1. **Zero-downtime deployments** with Docker and Traefik
2. **Cloud-native options** with Cloudflare and edge containers
3. **Automated CI/CD** with GitHub Actions
4. **Observability** with health checks and structured logging
5. **Disaster recovery** with automated backups
6. **Modern security** with Let's Encrypt and rate limiting
7. **Advanced WebSocket architecture** with Cloudflare Durable Objects (Chapter 9)
8. **Simple passthrough** with Cloudflare Tunnel (Chapter 10)

Choose the path that fits your infrastructure:
- **VPS + Docker**: Full control, predictable costs
- **Cloudflare + Fly.io**: Serverless, auto-scaling
- **Hybrid**: Best of both worlds
- **Durable Objects**: Maximum security and performance for high-scale apps (Chapter 9)
- **Direct Tunnel**: Simplicity and zero-code changes for moderate scale (Chapter 10)
