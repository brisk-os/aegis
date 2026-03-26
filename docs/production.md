# Running Aegis in Production

## Overview

Aegis is distributed as a Docker image tagged `aegis-latest` on Docker Hub. It requires a PostgreSQL database and a set of environment variables — no other infrastructure dependencies.

## Prerequisites

- Docker (or any container runtime)
- A PostgreSQL 16+ database
- An ES256 key pair (generated once, stored securely)

## 1. Pull the image

```bash
docker pull renjith/brisk:aegis-latest
```

## 2. Generate an ES256 key pair

Run this once and store the output in your secrets manager (AWS Secrets Manager, Doppler, etc.). **Never commit these values to source control.**

```bash
# Requires openssl
openssl ecparam -name prime256v1 -genkey -noout -out /tmp/aegis_ec.pem
openssl pkcs8 -topk8 -nocrypt -in /tmp/aegis_ec.pem -out /tmp/aegis_pkcs8.pem

# Base64-encode for use as env vars (no newlines or spaces)
JWT_PRIVATE_KEY=$(base64 < /tmp/aegis_pkcs8.pem | tr -d '\n')
JWT_PUBLIC_KEY=$(openssl ec -in /tmp/aegis_ec.pem -pubout 2>/dev/null | base64 | tr -d '\n')

echo "JWT_PRIVATE_KEY=$JWT_PRIVATE_KEY"
echo "JWT_PUBLIC_KEY=$JWT_PUBLIC_KEY"

rm /tmp/aegis_ec.pem /tmp/aegis_pkcs8.pem
```

If you have the Aegis repo checked out, `make keys` does the same thing.

The public key can be shared with consuming services so they can verify JWTs locally. The private key must stay on Aegis only.

## 3. Generate an admin key

The admin API (`POST /admin/applications`) is protected by a static secret. Generate one:

```bash
openssl rand -hex 32
```

Store this as `AEGIS_ADMIN_KEY` in your secrets manager.

## 4. Configure environment variables

| Variable | Required | Description |
|----------|----------|-------------|
| `DATABASE_URL` | Yes | `postgres://user:password@host:5432/dbname` |
| `JWT_PRIVATE_KEY` | Yes | Base64-encoded PKCS#8 PEM private key |
| `JWT_PUBLIC_KEY` | Yes | Base64-encoded SPKI PEM public key |
| `AEGIS_ADMIN_KEY` | Yes | Random secret for the admin API |
| `ACCESS_TOKEN_EXPIRY_SECS` | No | Access token lifetime (default: `900` — 15 min) |
| `REFRESH_TOKEN_EXPIRY_SECS` | No | Refresh token lifetime (default: `2592000` — 30 days) |
| `PORT` | No | HTTP port (default: `8080`) |
| `RUST_LOG` | No | Log filter, e.g. `aegis=info` |

## 5. Database

Aegis manages its own schema via SQLx migrations, which run automatically on startup. Point `DATABASE_URL` at a Postgres 16 instance and Aegis will create all tables on first boot.

```
DATABASE_URL=postgres://aegis:strongpassword@db.internal:5432/aegis
```

Ensure the database user has permission to create tables and indexes. On subsequent starts, only pending migrations run — existing data is untouched.

## 6. Run the container

```bash
docker run -d \
  --name aegis \
  --restart unless-stopped \
  -p 8080:8080 \
  -e DATABASE_URL="postgres://aegis:strongpassword@db.internal:5432/aegis" \
  -e JWT_PRIVATE_KEY="<base64-encoded-private-key>" \
  -e JWT_PUBLIC_KEY="<base64-encoded-public-key>" \
  -e AEGIS_ADMIN_KEY="<random-hex>" \
  -e RUST_LOG="aegis=info" \
  renjith/brisk:aegis-latest
```

Or with Docker Compose:

```yaml
services:
  aegis:
    image: renjith/brisk:aegis-latest
    restart: unless-stopped
    ports:
      - "8080:8080"
    environment:
      DATABASE_URL: postgres://aegis:strongpassword@db:5432/aegis
      JWT_PRIVATE_KEY: ${JWT_PRIVATE_KEY}
      JWT_PUBLIC_KEY: ${JWT_PUBLIC_KEY}
      AEGIS_ADMIN_KEY: ${AEGIS_ADMIN_KEY}
      RUST_LOG: aegis=info
    depends_on:
      db:
        condition: service_healthy

  db:
    image: postgres:16-alpine
    restart: unless-stopped
    environment:
      POSTGRES_USER: aegis
      POSTGRES_PASSWORD: strongpassword
      POSTGRES_DB: aegis
    volumes:
      - pgdata:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U aegis"]
      interval: 5s
      timeout: 5s
      retries: 5

volumes:
  pgdata:
```

## 7. Register your first application

Once Aegis is running, register each app that will use it. The `client_secret` is returned **only once** — store it immediately in your app's secrets manager.

```bash
curl -X POST http://localhost:8080/admin/applications \
  -H "Content-Type: application/json" \
  -H "X-Admin-Key: <AEGIS_ADMIN_KEY>" \
  -d '{"name": "my-app"}'
```

```json
{
  "id": "...",
  "client_id": "app_abc123",
  "client_secret": "secret_xyz...",
  "name": "my-app"
}
```

Give `client_id` and `client_secret` to the app's backend (BFF). They are used as HTTP Basic credentials on all subsequent requests to Aegis.

## 8. Health check

Aegis exposes a health-check-friendly endpoint:

```
GET /.well-known/jwks.json
```

A `200` response means the service is up and the JWT keys are loaded. Wire this into your load balancer or orchestrator's health check.

## Security checklist

- [ ] `JWT_PRIVATE_KEY` is stored in a secrets manager, not in env files or source control
- [ ] `AEGIS_ADMIN_KEY` is a long random string (minimum 32 hex chars) and is not shared with app BFFs
- [ ] Aegis is not directly reachable from the internet — only your BFF should call it
- [ ] The database user has the minimum required permissions (no superuser)
- [ ] Database traffic is over TLS (add `?sslmode=require` to `DATABASE_URL`)
- [ ] Access token expiry (`ACCESS_TOKEN_EXPIRY_SECS`) is kept short (15 min default is reasonable)
- [ ] Container runs with `--restart unless-stopped` or equivalent in your orchestrator
