# Better Auth Organization Best Practices

## Setup

1. Add `organization()` plugin to server config
2. Add `organizationClient()` plugin to client config
3. Run `npx @better-auth/cli@latest migrate` (built-in adapter) or generate + push for Drizzle/Prisma
4. Verify: check that organization, member, invitation tables exist in your database

```ts
import { betterAuth } from "better-auth";
import { organization } from "better-auth/plugins";

export const auth = betterAuth({
  plugins: [
    organization({
      allowUserToCreateOrganization: true,
      organizationLimit: 5,
      membershipLimit: 100,
    }),
  ],
});
```

### Client-Side Setup

```ts
import { createAuthClient } from "better-auth/client";
import { organizationClient } from "better-auth/client/plugins";

export const authClient = createAuthClient({
  plugins: [organizationClient()],
});
```

## Creating Organizations

The creator is automatically assigned the `owner` role.

```ts
const { data, error } = await authClient.organization.create({
  name: "My Company",
  slug: "my-company",
  logo: "https://example.com/logo.png",
  metadata: { plan: "pro" },
});
```

### Controlling Organization Creation

```ts
organization({
  allowUserToCreateOrganization: async (user) => {
    return user.emailVerified === true;
  },
  organizationLimit: async (user) => {
    return user.plan === "premium" ? 20 : 3;
  },
});
```

### Creating Organizations on Behalf of Users (Server-Side)

```ts
await auth.api.createOrganization({
  body: {
    name: "Client Organization",
    slug: "client-org",
    userId: "user-id-who-will-be-owner", // required; cannot be used with session headers
  },
});
```

## Active Organizations

Stored in the session and scopes subsequent API calls:

```ts
const { data, error } = await authClient.organization.setActive({
  organizationId,
});
```

Use `getFullOrganization()` to retrieve the active org with all members, invitations, and teams.

## Members

### Adding Members (Server-Side)

```ts
await auth.api.addMember({
  body: {
    userId: "user-id",
    role: "member",
    organizationId: "org-id",
  },
});
```

For client-side member additions, use the invitation system instead.

### Assigning Multiple Roles

```ts
await auth.api.addMember({
  body: {
    userId: "user-id",
    role: ["admin", "moderator"],
    organizationId: "org-id",
  },
});
```

### Removing Members

Use `removeMember({ memberIdOrEmail })`. The last owner cannot be removed — assign ownership to another member first.

### Membership Limits

```ts
organization({
  membershipLimit: async (user, organization) => {
    if (organization.metadata?.plan === "enterprise") return 1000;
    return 50;
  },
});
```

## Invitations

### Setting Up Invitation Emails

```ts
organization({
  sendInvitationEmail: async (data) => {
    const { email, organization, inviter, invitation } = data;
    await sendEmail({
      to: email,
      subject: `Join ${organization.name}`,
      html: `
        <p>${inviter.user.name} invited you to join ${organization.name}</p>
        <a href="https://yourapp.com/accept-invite?id=${invitation.id}">Accept Invitation</a>
      `,
    });
  },
});
```

### Sending Invitations

```ts
await authClient.organization.inviteMember({
  email: "newuser@example.com",
  role: "member",
});
```

### Shareable Invitation URLs

```ts
const { data } = await authClient.organization.getInvitationURL({
  email: "newuser@example.com",
  role: "member",
  callbackURL: "https://yourapp.com/dashboard",
});
// Share data.url via any channel
```

This does not call `sendInvitationEmail` — handle delivery yourself.

### Invitation Configuration

```ts
organization({
  invitationExpiresIn: 60 * 60 * 24 * 7, // 7 days (default: 48 hours)
  invitationLimit: 100,
  cancelPendingInvitationsOnReInvite: true,
});
```

## Roles & Permissions

Default roles: `owner` (full access), `admin` (manage members/invitations/settings), `member` (basic access).

### Checking Permissions

```ts
const { data } = await authClient.organization.hasPermission({
  permission: "member:write",
});
```

Use `checkRolePermission({ role, permissions })` for client-side UI rendering (static only). For dynamic access control, always use the `hasPermission` endpoint.

## Teams

### Enabling Teams

```ts
organization({
  teams: {
    enabled: true,
    maximumTeams: 20,
    maximumMembersPerTeam: 50,
    allowRemovingAllTeams: false,
  },
});
```

### Creating and Managing Teams

```ts
const { data } = await authClient.organization.createTeam({ name: "Engineering" });
```

Use `addTeamMember({ teamId, userId })` (member must be in org first), `removeTeamMember({ teamId, userId })` (stays in org), and `setActiveTeam({ teamId })`.

## Dynamic Access Control

```ts
import { organization } from "better-auth/plugins";
import { dynamicAccessControl } from "@better-auth/organization/addons";

organization({
  dynamicAccessControl: { enabled: true },
});
```

### Creating Custom Roles

```ts
await authClient.organization.createRole({
  role: "moderator",
  permission: {
    member: ["read"],
    invitation: ["read"],
  },
});
```

Use `updateRole({ roleId, permission })` and `deleteRole({ roleId })`. Pre-defined roles (owner, admin, member) and roles assigned to members cannot be deleted.

## Lifecycle Hooks

```ts
organization({
  hooks: {
    organization: {
      beforeCreate: async ({ data, user }) => {
        return {
          data: { ...data, metadata: { ...data.metadata, createdBy: user.id } },
        };
      },
      afterCreate: async ({ organization, member }) => {
        await createDefaultResources(organization.id);
      },
      beforeDelete: async ({ organization }) => {
        await archiveOrganizationData(organization.id);
      },
    },
    member: {
      afterCreate: async ({ member, organization }) => {
        await notifyAdmins(organization.id, `New member joined`);
      },
    },
    invitation: {
      afterCreate: async ({ invitation, organization, inviter }) => {
        await logInvitation(invitation);
      },
    },
  },
});
```

## Schema Customization

```ts
organization({
  schema: {
    organization: {
      modelName: "workspace",
      fields: { name: "workspaceName" },
      additionalFields: {
        billingId: { type: "string", required: false },
      },
    },
    member: {
      additionalFields: {
        department: { type: "string", required: false },
        title: { type: "string", required: false },
      },
    },
  },
});
```

## Security Considerations

### Owner Protection

- The last owner cannot be removed from or leave an organization
- The owner role cannot be removed from the last owner

Always transfer ownership before removing the current owner:

```ts
await authClient.organization.updateMemberRole({
  memberId: "new-owner-member-id",
  role: "owner",
});
```

### Organization Deletion

Deleting an organization removes all associated data. Prevent accidental deletion:

```ts
organization({
  disableOrganizationDeletion: true,
});
```

Or implement soft delete via hooks:

```ts
organization({
  hooks: {
    organization: {
      beforeDelete: async ({ organization }) => {
        await archiveOrganization(organization.id);
        throw new Error("Organization archived, not deleted");
      },
    },
  },
});
```

### Invitation Security

- Invitations expire after 48 hours by default
- Only the invited email address can accept an invitation
- Pending invitations can be cancelled by organization admins

## Complete Configuration Example

```ts
import { betterAuth } from "better-auth";
import { organization } from "better-auth/plugins";
import { sendEmail } from "./email";

export const auth = betterAuth({
  plugins: [
    organization({
      allowUserToCreateOrganization: true,
      organizationLimit: 10,
      membershipLimit: 100,
      creatorRole: "owner",
      defaultOrganizationIdField: "slug",
      invitationExpiresIn: 60 * 60 * 24 * 7,
      invitationLimit: 50,
      sendInvitationEmail: async (data) => {
        await sendEmail({
          to: data.email,
          subject: `Join ${data.organization.name}`,
          html: `<a href="https://app.com/invite/${data.invitation.id}">Accept</a>`,
        });
      },
      hooks: {
        organization: {
          afterCreate: async ({ organization }) => {
            console.log(`Organization ${organization.name} created`);
          },
        },
      },
    }),
  ],
});
```
