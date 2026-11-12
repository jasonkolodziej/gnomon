---
name: better-auth
description: Complete Better Auth integration guide. Configure server and client, set up database adapters, manage sessions, scaffold authentication, add OAuth providers, configure rate limiting, set up CSRF protection, secure sessions and cookies, implement email verification, password reset, organizations/teams, and two-factor authentication. Use when users mention Better Auth, betterauth, auth.ts, or need TypeScript authentication with email/password, OAuth, or plugin configuration.
---

# Better Auth Skill

**Always consult [better-auth.com/docs](https://better-auth.com/docs) for code examples and the latest API.**

---

## Reference Files

Load the relevant reference file(s) for the task at hand:

| Task                                                                                         | File                                                                 |
| -------------------------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| Initial setup, server config, database adapters, sessions, plugins, hooks                    | [references/core-setup.md](references/core-setup.md)                 |
| Scaffold auth from scratch, detect frameworks, plan and implement full auth                  | [references/create-auth.md](references/create-auth.md)               |
| Email verification, password reset, hashing, password policies                               | [references/email-and-password.md](references/email-and-password.md) |
| Rate limiting, CSRF, trusted origins, cookie security, OAuth token encryption, audit logging | [references/security.md](references/security.md)                     |
| Organizations, teams, member roles, RBAC, invitations                                        | [references/organization.md](references/organization.md)             |
| Two-factor authentication, TOTP, OTP, backup codes, trusted devices                          | [references/two-factor-auth.md](references/two-factor-auth.md)       |

---

## Quick Start

```bash
npm install better-auth
```

```env
BETTER_AUTH_SECRET=<generate: openssl rand -base64 32>
BETTER_AUTH_URL=http://localhost:3000
```

Verify install: `GET /api/auth/ok` → `{ status: "ok" }`

---

## Resources

- [Docs](https://better-auth.com/docs)
- [Options Reference](https://better-auth.com/docs/reference/options)
- [LLMs.txt](https://better-auth.com/llms.txt)
- [GitHub](https://github.com/better-auth/better-auth)
