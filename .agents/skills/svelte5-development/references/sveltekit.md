# SvelteKit — Detailed Reference

Comprehensive examples for SvelteKit routing, data loading, form actions, server-sent events, environment variables, and component patterns.

## Dynamic Routes

- `[slug]` — Single parameter
- `[...rest]` — Rest parameter (catches multiple segments)
- `[[optional]]` — Optional parameter

Example: `src/routes/blog/[slug]/+page.svelte` matches `/blog/hello-world`

---

## Loading Data

### Page Load

**JavaScript** (`+page.js`):

```js
/** @type {import('./$types').PageLoad} */
export function load({ params, url, fetch }) {
    return {
        post: {
            title: `Post ${params.slug}`,
            content: 'Content here',
        },
    };
}
```

**TypeScript** (`+page.ts`):

```ts
import type { PageLoad } from './$types';

export const load: PageLoad = ({ params, url, fetch }) => {
    return {
        post: {
            title: `Post ${params.slug}`,
            content: 'Content here',
        },
    };
};
```

### Layout Load

**JavaScript** (`+layout.server.js`):

```js
/** @type {import('./$types').LayoutServerLoad} */
export async function load() {
    return {
        sections: [
            { slug: 'profile', title: 'Profile' },
            { slug: 'settings', title: 'Settings' },
        ],
    };
}
```

**TypeScript** (`+layout.server.ts`):

```ts
import type { LayoutServerLoad } from './$types';

export const load: LayoutServerLoad = async () => {
    return {
        sections: [
            { slug: 'profile', title: 'Profile' },
            { slug: 'settings', title: 'Settings' },
        ],
    };
};
```

### Accessing Data in Components

**JavaScript**:

```svelte
<script>
	/** @type {import('./$types').PageProps} */
	let { data } = $props();
</script>

<h1>{data.post.title}</h1>
```

### Parent Data Access

**JavaScript**:

```js
export async function load({ parent }) {
    const { user } = await parent();
    return { username: user.name };
}
```

**TypeScript**:

```ts
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ parent }) => {
    const { user } = await parent();
    return { username: user.name };
};
```

---

## Universal vs Server Load

**Use `+page.js` (Universal) when**:

- Fetching from public APIs
- No private credentials needed
- Returning custom classes or constructors
- Running same logic server + client

**Use `+page.server.js` (Server) when**:

- Accessing database directly
- Using private environment variables
- Reading from filesystem
- Need to serialize data for client

**Server load return values must be serializable** (JSON + Date, Map, Set, RegExp, BigInt).

---

## Form Actions

**JavaScript** (`+page.server.js`):

```js
export const actions = {
    default: async ({ request }) => {
        const data = await request.formData();
        const email = data.get('email');
        // process form...
        return { success: true };
    },
};
```

**TypeScript** (`+page.server.ts`):

```ts
import type { Actions } from './$types';

export const actions: Actions = {
    default: async ({ request }) => {
        const data = await request.formData();
        const email = data.get('email');
        // process form...
        return { success: true };
    },
};
```

**Page component** (`+page.svelte`):

```svelte
<script>
	/** @type {import('./$types').PageProps} */
	let { form } = $props();
</script>

{#if form?.success}
	<p>Success!</p>
{/if}

<form method="POST">
	<input name="email" type="email" />
	<button>Submit</button>
</form>
```

---

## Real-time Updates (Server-Sent Events)

**Server (JavaScript)** (`+server.js`):

```js
export function GET() {
    const stream = new ReadableStream({
        start(controller) {
            const encoder = new TextEncoder();
            const send = (data) => {
                controller.enqueue(encoder.encode(`data: ${JSON.stringify(data)}\n\n`));
            };
            setInterval(() => send({ time: Date.now() }), 1000);
        },
    });

    return new Response(stream, {
        headers: {
            'Content-Type': 'text/event-stream',
            'Cache-Control': 'no-cache',
        },
    });
}
```

**Server (TypeScript)** (`+server.ts`):

```ts
import type { RequestHandler } from './$types';

export const GET: RequestHandler = () => {
    const stream = new ReadableStream<Uint8Array>({
        start(controller) {
            const encoder = new TextEncoder();
            const send = (data: unknown) => {
                controller.enqueue(encoder.encode(`data: ${JSON.stringify(data)}\n\n`));
            };
            setInterval(() => send({ time: Date.now() }), 1000);
        },
    });

    return new Response(stream, {
        headers: {
            'Content-Type': 'text/event-stream',
            'Cache-Control': 'no-cache',
        },
    });
};
```

**Client** (`+page.svelte`):

```svelte
<script>
	let data = $state({ time: 0 });

	$effect(() => {
		const source = new EventSource('/updates');
		source.onmessage = (e) => {
			data = JSON.parse(e.data);
		};
		return () => source.close();
	});
</script>
```

**TypeScript client**:

```svelte
<script lang="ts">
	let data = $state<{ time: number }>({ time: 0 });

	$effect(() => {
		const source = new EventSource('/updates');
		source.onmessage = (e: MessageEvent<string>) => {
			data = JSON.parse(e.data) as { time: number };
		};
		return () => source.close();
	});
</script>
```

---

## Environment Variables

**Public** (safe to expose to client):

```js
import { PUBLIC_STATION_NAME } from '$env/static/public';
```

**Private** (server-only):

```js
import { DATABASE_URL } from '$env/static/private';
```

---

## Component Patterns

### Snippets and Render

Reusable markup patterns within a component:

```svelte
<script>
	let items = $state(['a', 'b', 'c']);
</script>

{#snippet listItem(item)}
	<li>{item}</li>
{/snippet}

<ul>
	{#each items as item}
		{@render listItem(item)}
	{/each}
</ul>
```

### Conditional Rendering

```svelte
{#if condition}
	<p>True</p>
{:else if otherCondition}
	<p>Other</p>
{:else}
	<p>False</p>
{/if}
```

### Lists with Keys

```svelte
{#each items as item, index (item.id)}
	<div>{index}: {item.name}</div>
{/each}
```

**Always use keys** (the `(item.id)` part) for dynamic lists.

### Async/Await (Svelte 5.36+)

Requires `experimental.async: true` in `svelte.config.js`:

```svelte
<script>
	async function fetchData() {
		const res = await fetch('/api/data');
		return res.json();
	}
</script>

<p>{await fetchData()}</p>

<!-- Or with #await for loading/error states -->
{#await fetchData()}
	Loading...
{:then data}
	<p>{data.message}</p>
{:catch error}
	<p>Error: {error.message}</p>
{/await}
```
