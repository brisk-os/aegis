# aegis

Headless, multi-tenant authentication service. Built with Rust, Axum, and PostgreSQL.

Acts as the auth backend for multiple applications — similar in spirit to Clerk or Kinde, but self-hosted. Each application is an isolated tenant: users, organizations, and credentials are fully scoped per app.

## Documentation

- [Running in production](docs/production.md)

## How it works

Every application registers with Aegis and receives a `client_id` + `client_secret`. Backend services (BFFs) authenticate all requests to Aegis using HTTP Basic auth with those credentials. Aegis never talks directly to a browser or mobile client — all calls come through the app's backend.

```
Mobile / Web  →  App Backend (BFF)  →  Aegis
                 holds client_secret     issues JWTs
```

On signup, Aegis automatically creates a **personal organization** for the user within that app. Team organizations can be created on top of this. The issued JWT always carries `org_id`, `org_type`, and `role` so the consuming app has full access context without a second round-trip.

## Prerequisites

- [Docker](https://docs.docker.com/get-docker/) and [Docker Compose](https://docs.docker.com/compose/) (v2)
- An ES256 key pair (see [Key generation](#key-generation))

> No local Rust or PostgreSQL installation required — everything runs in containers.

## Quick start

```bash
# 1. Generate an ES256 key pair
make keys

# 2. Copy the output into a .env file
cp .env.example .env
# Edit .env and paste in JWT_PRIVATE_KEY and JWT_PUBLIC_KEY

# 3. Start the database and API
make up
make logs
```

The API is available at **http://localhost:8080** once `make up` completes.

## Key generation

Aegis uses ES256 (ECDSA P-256) for JWTs. Run:

```bash
make keys
```

This prints two `export` lines ready to paste into `.env`:

```
JWT_PRIVATE_KEY=-----BEGIN EC PRIVATE KEY-----\n...\n-----END EC PRIVATE KEY-----\n
JWT_PUBLIC_KEY=-----BEGIN PUBLIC KEY-----\n...\n-----END PUBLIC KEY-----\n
```

Keep `JWT_PRIVATE_KEY` secret. `JWT_PUBLIC_KEY` can be shared — consuming services use it to verify tokens locally or via `GET /.well-known/jwks.json`.

## Running the integration tests

### Full isolated run (recommended for CI)

```bash
make test-clean
```

This runs the complete lifecycle in one command:
1. Wipes any existing DB volume
2. Rebuilds and starts a fresh DB + API (`--wait` blocks until the healthcheck passes)
3. Runs the Vitest integration-test suite inside a Docker container
4. Tears everything down, leaving no leftover state

### Run tests against already-running containers

```bash
make up
make test
```

## All commands

| Command | Description |
|---------|-------------|
| `make up` | Build images and start DB + API in the background |
| `make down` | Stop containers, keep DB volume |
| `make clean` | Stop containers and wipe the DB volume |
| `make test` | Run tests against already-running containers |
| `make test-clean` | Full isolated test run (wipe → start → test → teardown) |
| `make logs` | Tail API logs |
| `make db-shell` | Open a `psql` shell into the running database |
| `make keys` | Generate a fresh ES256 key pair |

## Environment variables

| Variable | Required | Description |
|----------|----------|-------------|
| `DATABASE_URL` | Yes | Postgres connection string |
| `JWT_PRIVATE_KEY` | Yes | ES256 PEM private key (newlines as `\n`) |
| `JWT_PUBLIC_KEY` | Yes | ES256 PEM public key (newlines as `\n`) |
| `AEGIS_ADMIN_KEY` | Yes | Static key for the admin API (`X-Admin-Key` header) |
| `ACCESS_TOKEN_EXPIRY_SECS` | No | Access token lifetime in seconds (default: `900`) |
| `REFRESH_TOKEN_EXPIRY_SECS` | No | Refresh token lifetime in seconds (default: `2592000`) |
| `PORT` | No | HTTP port (default: `8080`) |
| `RUST_LOG` | No | Log filter, e.g. `aegis=debug` |

## API reference

### Admin

These endpoints are protected by the `X-Admin-Key` header.

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/admin/applications` | Register a new application |

**Create application**

```bash
curl -X POST http://localhost:8080/admin/applications \
  -H "Content-Type: application/json" \
  -H "X-Admin-Key: <admin_key>" \
  -d '{"name": "my-app"}'
```

Response (the `client_secret` is only returned once — store it securely):

```json
{
  "id": "...",
  "client_id": "app_abc123",
  "client_secret": "secret_xyz...",
  "name": "my-app"
}
```

### Auth

All `/auth/*`, `/users`, `/orgs`, and `/orgs/:id/members` endpoints require HTTP Basic auth:

```
Authorization: Basic base64(client_id:client_secret)
```

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/auth/signup` | Create a new user (auto-creates a personal org) |
| `POST` | `/auth/login` | Authenticate a user, returns access + refresh tokens |
| `POST` | `/auth/refresh` | Rotate refresh token, returns a new token pair |
| `POST` | `/auth/logout` | Revoke a refresh token |

**Signup**

```bash
curl -X POST http://localhost:8080/auth/signup \
  -H "Authorization: Basic $(echo -n 'app_abc123:secret_xyz' | base64)" \
  -H "Content-Type: application/json" \
  -d '{"name": "Alice", "email": "alice@example.com", "password": "hunter2secure"}'
```

**Login**

```bash
curl -X POST http://localhost:8080/auth/login \
  -H "Authorization: Basic $(echo -n 'app_abc123:secret_xyz' | base64)" \
  -H "Content-Type: application/json" \
  -d '{"email": "alice@example.com", "password": "hunter2secure"}'
```

```json
{
  "access_token": "<jwt>",
  "refresh_token": "<jwt>",
  "token_type": "Bearer"
}
```

The access token payload includes:

```json
{
  "sub": "<user_id>",
  "app_id": "<app_id>",
  "org_id": "<personal_org_id>",
  "org_type": "personal",
  "role": "owner",
  "email": "alice@example.com",
  "jti": "<uuid>",
  "iat": 1234567890,
  "exp": 1234568790
}
```

### Users

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/users` | List all users in this application |
| `GET` | `/users/:id` | Get a user by ID |

### Organizations

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/orgs` | List all organizations in this application |
| `POST` | `/orgs` | Create a new team organization |
| `GET` | `/orgs/:id` | Get an organization by ID |

### Members

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/orgs/:id/members` | List members of an organization |
| `POST` | `/orgs/:id/members` | Add a user to an organization |
| `DELETE` | `/orgs/:id/members/:user_id` | Remove a user from an organization |

### JWKS

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/.well-known/jwks.json` | Public key set for JWT verification |

Consuming services can fetch this endpoint to verify Aegis-issued JWTs locally without making a round-trip for every request.

## Security model

- **App isolation:** Users, organizations, and credentials are scoped to an `app_id`. A `client_secret` is required to access any app's data — one app cannot read another app's users even if it knows the other app's `client_id`.
- **Passwords:** Hashed with Argon2 (memory-hard, resistant to brute-force).
- **JWTs:** Signed with ES256 (asymmetric). Only Aegis holds the private key; consuming services verify with the public key.
- **Refresh token rotation:** Each use of a refresh token invalidates it and issues a new one. Re-use of a rotated token returns `401`.
- **Admin key:** The `/admin/*` routes are protected by a static `X-Admin-Key`. Set this to a long random string in production and keep it out of your BFF code.

## Local development (without Docker)

Requires Rust stable and a running PostgreSQL instance.

```bash
cp .env.example .env
# Fill in DATABASE_URL, JWT_PRIVATE_KEY, JWT_PUBLIC_KEY, AEGIS_ADMIN_KEY

cargo run       # runs migrations automatically, starts on :8080
cargo check     # fast type-check without full build
cargo build     # full build
```

Change all secrets in `.env` before deploying to production.
