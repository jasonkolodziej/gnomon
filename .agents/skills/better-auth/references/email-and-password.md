# Better Auth Email & Password Best Practices

## Quick Start

1. Enable email/password: `emailAndPassword: { enabled: true }`
2. Configure `emailVerification.sendVerificationEmail`
3. Add `sendResetPassword` for password reset flows
4. Run `npx @better-auth/cli@latest migrate`
5. Verify: attempt sign-up and confirm verification email triggers

---

## Email Verification Setup

```ts
import { betterAuth } from "better-auth";
import { sendEmail } from "./email";

export const auth = betterAuth({
  emailVerification: {
    sendVerificationEmail: async ({ user, url, token }, request) => {
      await sendEmail({
        to: user.email,
        subject: "Verify your email address",
        text: `Click the link to verify your email: ${url}`,
      });
    },
  },
});
```

The `url` parameter contains the full verification link. The `token` is available if you need to build a custom verification URL.

### Requiring Email Verification

Blocks sign-in until the user verifies their email. Unverified users receive a new verification email on each sign-in attempt.

```ts
export const auth = betterAuth({
  emailAndPassword: {
    requireEmailVerification: true,
  },
});
```

Requires `sendVerificationEmail` to be configured. Only applies to email/password sign-ins.

## Client Side Validation

Implement client-side validation for immediate user feedback and reduced server load.

## Callback URLs

Always use absolute URLs (including the origin) for callback URLs:

```ts
const { data, error } = await authClient.signUp.email({
  callbackURL: "https://example.com/callback",
});
```

## Password Reset Flows

```ts
import { betterAuth } from "better-auth";
import { sendEmail } from "./email";

export const auth = betterAuth({
  emailAndPassword: {
    enabled: true,
    sendResetPassword: async ({ user, url, token }, request) => {
      void sendEmail({
        to: user.email,
        subject: "Reset your password",
        text: `Click the link to reset your password: ${url}`,
      });
    },
    onPasswordReset: async ({ user }, request) => {
      console.log(`Password for user ${user.email} has been reset.`);
    },
  },
});
```

### Security Considerations

Built-in protections: background email sending (timing attack prevention), dummy operations on invalid requests, constant response messages regardless of user existence.

On serverless platforms, configure a background task handler:

```ts
export const auth = betterAuth({
  advanced: {
    backgroundTasks: {
      handler: (promise) => {
        // Use platform-specific methods like waitUntil
        waitUntil(promise);
      },
    },
  },
});
```

#### Token Security

Tokens expire after 1 hour by default. Configure with `resetPasswordTokenExpiresIn` (in seconds):

```ts
export const auth = betterAuth({
  emailAndPassword: {
    enabled: true,
    resetPasswordTokenExpiresIn: 60 * 30, // 30 minutes
  },
});
```

Tokens are single-use — deleted immediately after successful reset.

#### Session Revocation

```ts
export const auth = betterAuth({
  emailAndPassword: {
    enabled: true,
    revokeSessionsOnPasswordReset: true,
  },
});
```

#### Password Requirements

```ts
export const auth = betterAuth({
  emailAndPassword: {
    enabled: true,
    minPasswordLength: 12,
    maxPasswordLength: 256,
  },
});
```

### Sending the Password Reset

```ts
// Server-side
const data = await auth.api.requestPasswordReset({
  body: {
    email: "john.doe@example.com",
    redirectTo: "https://example.com/reset-password",
  },
});
```

```ts
// Client-side
const { data, error } = await authClient.requestPasswordReset({
  email: "john.doe@example.com",
  redirectTo: "https://example.com/reset-password",
});
```

## Password Hashing

Default: `scrypt` (Node.js native, no external dependencies).

### Custom Hashing Algorithm

To use Argon2id:

```ts
import { betterAuth } from "better-auth";
import { hash, verify, type Options } from "@node-rs/argon2";

const argon2Options: Options = {
  memoryCost: 65536, // 64 MiB
  timeCost: 3,
  parallelism: 4,
  outputLen: 32,
  algorithm: 2, // Argon2id
};

export const auth = betterAuth({
  emailAndPassword: {
    enabled: true,
    password: {
      hash: (password) => hash(password, argon2Options),
      verify: ({ password, hash: storedHash }) =>
        verify(storedHash, password, argon2Options),
    },
  },
});
```

If you switch hashing algorithms on an existing system, users with passwords hashed using the old algorithm won't be able to sign in. Plan a migration strategy if needed.
