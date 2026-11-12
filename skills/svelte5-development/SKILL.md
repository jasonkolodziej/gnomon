---
name: svelte5-development
description: Comprehensive Svelte 5 and SvelteKit development guidance. Use this skill when building Svelte components, working with runes, or developing SvelteKit applications. Covers reactive patterns, component architecture, routing, and data loading.
compatibility: Requires Node.js and a SvelteKit project (npm create svelte@latest)
metadata:
  version: "1.0"
---

This skill provides guidance for Svelte 5 and SvelteKit development, covering runes, component patterns,
routing, and common pitfalls.

For detailed code examples, see:

- [references/runes.md](references/runes.md) — in-depth rune examples with JS + TS variants
- [references/sveltekit.md](references/sveltekit.md) — SvelteKit routing, loading, forms, SSE, env vars
- [references/runtime-warnings.md](references/runtime-warnings.md) — every Svelte 5 runtime warning with ❌/✅ before-after fixes

## JavaScript and TypeScript Equivalents

Use both styles when sharing examples:

- Prefer paired examples labeled **JavaScript** and **TypeScript**.
- For Svelte components, use `<script>` for JavaScript and `<script lang="ts">` for TypeScript.
- For SvelteKit modules, pair `+page.js` with `+page.ts`, `+page.server.js` with `+page.server.ts`, etc.
- Keep behavior identical across both variants; only type annotations and file extensions should differ.

## Svelte 5 Runes — Core Reactivity

### $state — Reactive State

Creates reactive state that updates the UI when changed. Objects and arrays become **deeply reactive proxies**.

```svelte
<script>
	let count = $state(0);
	let user = $state({ name: 'Alice', age: 30 });
</script>

<button onclick={() => count++}>Clicks: {count}</button>
```

**Key rules**:

- Destructuring breaks reactivity — always access state through the original variable.
- Use `$state.raw()` for large, non-reactive objects to avoid proxy overhead.
- `$state` can be used in class fields.

See [references/runes.md](references/runes.md) for deep reactivity, class usage, and `$state.raw` examples.

### $derived — Computed Values

Creates values that automatically update when dependencies change. **Never update other state inside `$derived`.**

```svelte
<script>
	let count = $state(0);
	let doubled = $derived(count * 2);
</script>
```

Use `$derived.by(() => ...)` for complex multi-step derivations.

### $effect — Side Effects

Runs after component mounts and after reactive state changes. Use for DOM manipulation, third-party libraries,
analytics, and subscriptions.

```svelte
<script>
	let size = $state(50);
	let canvas;

	$effect(() => {
		const ctx = canvas.getContext('2d');
		ctx.clearRect(0, 0, canvas.width, canvas.height);
		ctx.fillRect(0, 0, size, size);
		// return a cleanup function if needed:
		// return () => cleanup();
	});
</script>
```

**CRITICAL — when NOT to use `$effect`**:

```svelte
<!-- ❌ BAD — Don't synchronize state with $effect -->
<script>
	let count = $state(0);
	let doubled = $state();
	$effect(() => { doubled = count * 2; }); // WRONG — use $derived
</script>

<!-- ✅ GOOD -->
<script>
	let count = $state(0);
	let doubled = $derived(count * 2);
</script>
```

**Tracking rules**: Only synchronous reads inside the effect body are tracked. Reads inside `setTimeout`,
`setInterval`, or after `await` are NOT tracked.

Use `$effect.pre` (rare) when you need to run logic _before_ DOM updates.

### $props — Component Props

Receives data from parent components via destructuring.

```svelte
<!-- Child.svelte -->
<script>
	let { message, count = 0 } = $props();
</script>
```

**Important**: Props update reactively, but you should NOT mutate them directly (unless `$bindable`).

### $bindable — Two-Way Binding

Allows a child component to write back to a parent's state.

```svelte
<!-- FancyInput.svelte -->
<script>
	let { value = $bindable(), ...props } = $props();
</script>
<input bind:value {...props} />
```

**Use sparingly** — prefer callback props over `$bindable` in most cases.

## Common Patterns and Pitfalls

### Effect vs Derived

```svelte
<script>
	let a = $state(1);
	let b = $state(2);

	// ✅ GOOD — Use $derived for computed values
	let sum = $derived(a + b);

	// ✅ GOOD — Use $effect for side effects only
	$effect(() => {
		analytics.track('calculation', { sum });
	});
</script>
```

### Navigation with Remote Functions

When using SvelteKit's `remoteFunctions` feature, standard `<a href>` links don't trigger client-side navigation.

```svelte
<script>
	import { goto } from '$app/navigation';
</script>

<!-- ❌ DON'T use regular links with remote functions -->
<a href="/songs/{id}/edit">{title}</a>

<!-- ✅ DO use goto() -->
<button onclick={() => goto(`/songs/${id}/edit`)}>{title}</button>
```

### Async Form Initialization

```svelte
<script>
	let song = $state({ current: null });
	let title = $state('');
	let initialized = $state(false);

	$effect(() => {
		if (song.current && !initialized) {
			title = song.current.title || '';
			initialized = true;
		}
	});
</script>
```

## SvelteKit Routing

### File Structure

| File                               | Purpose                               |
| ---------------------------------- | ------------------------------------- |
| `+page.svelte`                     | Page component                        |
| `+page.js`                         | Universal load (runs server + client) |
| `+page.server.js`                  | Server-only load                      |
| `+layout.svelte`                   | Layout component (wraps pages)        |
| `+layout.js` / `+layout.server.js` | Layout load functions                 |
| `+error.svelte`                    | Error page                            |
| `+server.js`                       | API endpoints                         |

**Dynamic route segments**: `[slug]`, `[...rest]`, `[[optional]]`

For complete load function examples, form actions, server-sent events, and environment variables, see
[references/sveltekit.md](references/sveltekit.md).

### Universal vs Server Load — Quick Rule

- **`+page.js`** — public APIs, no secrets, custom class returns, shared server/client logic
- **`+page.server.js`** — database, private env vars, filesystem, must return serializable data

## Runtime Warnings

When a Svelte 5 runtime warning appears, identify the warning code and look it up in
[references/runtime-warnings.md](references/runtime-warnings.md). Common ones to know:

- **`derived_inert`** — `$derived` created inside `$effect`; move it to top level
- **`ownership_invalid_mutation`** — child mutating a prop without `$bindable`; use `$bindable()` or callbacks
- **`console_log_state`** — `console.log($state proxy)`; use `$inspect(...)` or `$state.snapshot(...)`
- **`await_reactivity_loss`** — state read after `await` inside a function; pass as arguments instead
- **`hydration_mismatch`** — SSR/CSR DOM differs; keep structure identical, toggle content after `onMount`

## Best Practices

1. **Use `$derived`, not `$effect`, for computed values**
2. **Don't mutate props** — use callbacks or `$bindable`
3. **Use `$effect.pre` only when necessary** (before DOM updates)
4. **Always provide keys in `{#each}` blocks** for dynamic lists: `{#each items as item (item.id)}`
5. **Prefer server load functions** for sensitive data
6. **Use `goto()` for navigation** when remote functions are enabled
7. **Provide JS and TS parity** in docs and snippets (same behavior, different typing)
8. **Type your components** with `PageProps`, `LayoutProps`, etc.

## When to Consult External Resources

This skill covers the most common Svelte 5 and SvelteKit patterns. For advanced topics, refer to official docs:

- SvelteKit adapters and deployment
- Advanced routing (route groups, breaking layouts)
- Hooks (`handle`, `handleFetch`, `handleError`)
- Service workers and offline support
- Custom Svelte stores (when not using runes)
- Migration from Svelte 4
- Testing strategies
- TypeScript advanced types
