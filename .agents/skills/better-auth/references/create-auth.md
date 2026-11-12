# Create Auth — Scaffold Better Auth in TypeScript/JavaScript Apps

Guide for adding authentication to TypeScript/JavaScript applications using Better Auth.

**For code examples and syntax, see [better-auth.com/docs](https://better-auth.com/docs).**

---

## Phase 1: Planning (REQUIRED before implementation)

Before writing any code, gather requirements by scanning the project and asking the user structured questions.

### Step 1: Scan the project

Analyze the codebase to auto-detect:
- **Framework** — Look for `next.config`, `svelte.config`, `nuxt.config`, `astro.config`, `vite.config`, or Express/Hono entry files.
- **Database/ORM** — Look for `prisma/schema.prisma`, `drizzle.config.ts`, `package.json` deps (`pg`, `postgres`, `@neondatabase/serverless`, `mysql2`, `better-sqlite3`, `mongoose`, `mongodb`). If `drizzle.config.ts` exists, read its `dialect` field to determine the DB type. Also check which Drizzle driver is installed (`drizzle-orm/node-postgres` → `pg`, `drizzle-orm/postgres-js` → `postgres`, `drizzle-orm/neon-http` → Neon).
- **Existing auth** — Look for `next-auth`, `lucia`, `clerk`, `supabase/auth`, `firebase/auth` in `package.json` or imports.
- **Package manager** — Check for `pnpm-lock.yaml`, `yarn.lock`, `bun.lockb`, or `package-lock.json`.

Use what you find to pre-fill defaults and skip questions you can already answer.

### Step 2: Ask planning questions

Use the `AskQuestion` tool to ask the user **all applicable questions in a single call**. Skip any question you already have a confident answer for.

**Questions to ask:**

1. **Project type** (skip if detected)
   - Options: New project from scratch | Adding auth to existing project | Migrating from another auth library

2. **Framework** (skip if detected)
   - Options: Next.js (App Router) | Next.js (Pages Router) | SvelteKit | Nuxt | Astro | Express | Hono | SolidStart | Other

3. **Database & ORM** (skip if detected)
   - Options: PostgreSQL (Prisma) | PostgreSQL (Drizzle) | PostgreSQL (pg driver) | MySQL (Prisma) | MySQL (Drizzle) | MySQL (mysql2 driver) | SQLite (Prisma) | SQLite (Drizzle) | SQLite (better-sqlite3 driver) | MongoDB (Mongoose) | MongoDB (native driver)

4. **Authentication methods** (always ask, allow multiple)
   - Options: Email & password | Social OAuth (Google, GitHub, etc.) | Magic link (passwordless email) | Passkey (WebAuthn) | Phone number

5. **Social providers** (only if Social OAuth selected, ask in follow-up)
   - Options: Google | GitHub | Apple | Microsoft | Discord | Twitter/X

6. **Email verification** (only if Email & password selected, ask in follow-up)
   - Options: Yes | No

7. **Email provider** (only if verification is Yes or password reset needed, ask in follow-up)
   - Options: Resend | Mock it for now (console.log)

8. **Features & plugins** (always ask, allow multiple)
   - Options: Two-factor authentication (2FA) | Organizations / teams | Admin dashboard | API bearer tokens | Password reset | None of these

9. **Auth pages** (always ask, allow multiple)
   - Always available: Sign in | Sign up
   - If Email & password: Forgot password | Reset password
   - If email verification enabled: Email verification

10. **Auth UI style** (always ask)
    - Options: Minimal & clean | Centered card with background | Split layout (form + hero image) | Floating / glassmorphism | Other

### Step 3: Summarize the plan

Present a concise implementation plan as a markdown checklist. Ask the user to confirm before proceeding to Phase 2.

---

## Phase 2: Implementation

Only proceed after the user confirms the plan.

```
Is this a new/empty project?
├─ YES → New project setup
│   1. Install better-auth (+ scoped packages per plan)
│   2. Create auth.ts with all planned config
│   3. Create auth-client.ts with framework client
│   4. Set up route handler
│   5. Set up environment variables
│   6. Run CLI migrate/generate
│   7. Add plugins from plan
│   8. Create auth UI pages
│
├─ MIGRATING → Migration from existing auth
│   1. Audit current auth for gaps
│   2. Plan incremental migration
│   3. Install better-auth alongside existing auth
│   4. Migrate routes, then session logic, then UI
│   5. Remove old auth library
│
└─ ADDING → Add auth to existing project
    1. Analyze project structure
    2. Install better-auth
    3. Create auth config matching plan
    4. Add route handler
    5. Run schema migrations
    6. Integrate into existing pages
    7. Add planned plugins and features
```

---

## Installation

**Core:** `npm install better-auth`

**Scoped packages (as needed):**
| Package | Use case |
|---------|----------|
| `@better-auth/passkey` | WebAuthn/Passkey auth |
| `@better-auth/sso` | SAML/OIDC enterprise SSO |
| `@better-auth/stripe` | Stripe payments |
| `@better-auth/scim` | SCIM user provisioning |
| `@better-auth/expo` | React Native/Expo |

---

## Environment Variables

```env
BETTER_AUTH_SECRET=<32+ chars, generate with: openssl rand -base64 32>
BETTER_AUTH_URL=http://localhost:3000
DATABASE_URL=<your database connection string>
```

Add OAuth secrets as needed: `GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET`, `GOOGLE_CLIENT_ID`, etc.

---

## Server Config (auth.ts)

**Location:** `lib/auth.ts` or `src/lib/auth.ts`

- **Minimal:** `database` + `emailAndPassword: { enabled: true }`
- **Standard adds:** `socialProviders`, `emailVerification.sendVerificationEmail`, `emailAndPassword.sendResetPassword`
- **Full adds:** `plugins`, `session`, `account.accountLinking`, `rateLimit`

**Export types:** `export type Session = typeof auth.$Infer.Session`

---

## Client Config (auth-client.ts)

**Import by framework:**
| Framework | Import |
|-----------|--------|
| React/Next.js | `better-auth/react` |
| Vue | `better-auth/vue` |
| Svelte | `better-auth/svelte` |
| Solid | `better-auth/solid` |
| Vanilla JS | `better-auth/client` |

Client plugins go in `createAuthClient({ plugins: [...] })`.

---

## Route Handler Setup

| Framework | File | Handler |
|-----------|------|---------|
| Next.js App Router | `app/api/auth/[...all]/route.ts` | `toNextJsHandler(auth)` → export `{ GET, POST }` |
| Next.js Pages | `pages/api/auth/[...all].ts` | `toNextJsHandler(auth)` → default export |
| Express | Any file | `app.all("/api/auth/*", toNodeHandler(auth))` |
| SvelteKit | `src/hooks.server.ts` | `svelteKitHandler(auth)` |
| SolidStart | Route file | `solidStartHandler(auth)` |
| Hono | Route file | `auth.handler(c.req.raw)` |

**Next.js Server Components:** Add `nextCookies()` plugin to auth config.

---

## Database Migrations

| Adapter | Command |
|---------|---------|
| Built-in Kysely | `npx @better-auth/cli@latest migrate` |
| Prisma | `npx @better-auth/cli@latest generate --output prisma/schema.prisma` then `npx prisma migrate dev` |
| Drizzle (dev) | `npx @better-auth/cli@latest generate --output src/db/auth-schema.ts` then `npx drizzle-kit push` |
| Drizzle (prod) | `npx @better-auth/cli@latest generate --output src/db/auth-schema.ts` then `npx drizzle-kit generate` then `npx drizzle-kit migrate` |

> `drizzle-kit push` skips migration files — only safe for development.

**Re-run after adding plugins.**

---

## Database Adapters

| Database | Setup |
|----------|-------|
| SQLite | Pass `better-sqlite3` or `bun:sqlite` instance directly |
| PostgreSQL | Pass `pg.Pool` instance directly |
| MySQL | Pass `mysql2` pool directly |
| Prisma | `prismaAdapter(prisma, { provider: "postgresql" })` from `better-auth/adapters/prisma` |
| Drizzle (pg) | `drizzleAdapter(db, { provider: "pg" })` from `better-auth/adapters/drizzle` |
| Drizzle (mysql) | `drizzleAdapter(db, { provider: "mysql" })` from `better-auth/adapters/drizzle` |
| Drizzle (sqlite) | `drizzleAdapter(db, { provider: "sqlite" })` from `better-auth/adapters/drizzle` |
| MongoDB | `mongodbAdapter(db)` from `better-auth/adapters/mongodb` |

### Drizzle + PostgreSQL Setup

```ts
// Option 1: node-postgres (pg)
import { drizzle } from "drizzle-orm/node-postgres"
import { Pool } from "pg"
import * as schema from "./auth-schema"

const pool = new Pool({ connectionString: process.env.DATABASE_URL })
export const db = drizzle(pool, { schema })
```

```ts
// Option 2: postgres.js
import { drizzle } from "drizzle-orm/postgres-js"
import postgres from "postgres"
import * as schema from "./auth-schema"

const client = postgres(process.env.DATABASE_URL!)
export const db = drizzle(client, { schema })
```

```ts
// Option 3: Neon serverless
import { drizzle } from "drizzle-orm/neon-http"
import { neon } from "@neondatabase/serverless"
import * as schema from "./auth-schema"

const sql = neon(process.env.DATABASE_URL!)
export const db = drizzle(sql, { schema })
```

Then pass to Better Auth:

```ts
import { betterAuth } from "better-auth"
import { drizzleAdapter } from "better-auth/adapters/drizzle"
import { db } from "./db"

export const auth = betterAuth({
  database: drizzleAdapter(db, { provider: "pg" }),
})
```

### Drizzle Config (`drizzle.config.ts`)

```ts
import { defineConfig } from "drizzle-kit"

export default defineConfig({
  schema: "./src/db/auth-schema.ts",
  out: "./drizzle",
  dialect: "postgresql",
  dbCredentials: {
    url: process.env.DATABASE_URL!,
  },
})
```

---

## Common Plugins

| Plugin | Server Import | Client Import | Purpose |
|--------|---------------|---------------|---------|
| `twoFactor` | `better-auth/plugins` | `twoFactorClient` | 2FA with TOTP/OTP |
| `organization` | `better-auth/plugins` | `organizationClient` | Teams/orgs |
| `admin` | `better-auth/plugins` | `adminClient` | User management |
| `bearer` | `better-auth/plugins` | - | API token auth |
| `openAPI` | `better-auth/plugins` | - | API docs |
| `passkey` | `@better-auth/passkey` | `passkeyClient` | WebAuthn |
| `sso` | `@better-auth/sso` | - | Enterprise SSO |

---

## Auth UI Implementation

**Sign in flow:**
1. `signIn.email({ email, password })` or `signIn.social({ provider, callbackURL })`
2. Handle `error` in response
3. Redirect on success

**Session check (client):** `useSession()` hook returns `{ data: session, isPending }`

**Session check (server):** `auth.api.getSession({ headers: await headers() })`

**Protected routes:** Check session, redirect to `/sign-in` if null.

---

## Security Checklist

- [ ] `BETTER_AUTH_SECRET` set (32+ chars)
- [ ] `advanced.useSecureCookies: true` in production
- [ ] `trustedOrigins` configured
- [ ] Rate limits enabled
- [ ] Email verification enabled
- [ ] Password reset implemented
- [ ] 2FA for sensitive apps
- [ ] CSRF protection NOT disabled
- [ ] `account.accountLinking` reviewed

---

## Troubleshooting

| Issue | Fix |
|-------|-----|
| "Secret not set" | Add `BETTER_AUTH_SECRET` env var |
| "Invalid Origin" | Add domain to `trustedOrigins` |
| Cookies not setting | Check `baseURL` matches domain; enable secure cookies in prod |
| OAuth callback errors | Verify redirect URIs in provider dashboard |
| Type errors after adding plugin | Re-run CLI generate/migrate |

---

## Resources

- [Docs](https://better-auth.com/docs)
- [Examples](https://github.com/better-auth/examples)
- [Plugins](https://better-auth.com/docs/concepts/plugins)
- [CLI](https://better-auth.com/docs/concepts/cli)
- [Migration Guides](https://better-auth.com/docs/guides)
