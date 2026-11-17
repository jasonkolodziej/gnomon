# Svelte 5 Runtime Warnings — Detailed Reference

Copy-paste-ready examples for every Svelte 5 runtime warning (client + shared). Each entry includes
the exact warning message, a ❌ problematic example, a ✅ corrected example, and root-cause explanation.

Both **JavaScript** and **TypeScript** variants are provided where applicable.

---

## Quick Reference Table

| Warning | Category | Fix Strategy |
|---------|----------|-------------|
| `assignment_value_stale` | Client | Separate `??=` assignment from mutation into two statements |
| `await_reactivity_loss` | Client | Pass reactive values as function arguments, not closures |
| `await_waterfall` | Client | Create promises first, then await them in parallel |
| `binding_property_non_reactive` | Client + Shared | Ensure bound property is declared with `$state` or `$bindable` |
| `console_log_state` | Client | Use `$inspect(...)` or `$state.snapshot(...)` |
| `derived_inert` | Client | Create `$derived` at top level or inside `$effect.root` |
| `event_handler_invalid` | Client | Pass function reference, not invocation result |
| `hydratable_missing_but_expected` | Client | Ensure hydratables render on both server and client |
| `hydration_attribute_changed` | Client | Force update after mount, or ensure SSR/CSR values match |
| `hydration_html_changed` | Client | Force update after mount, or ensure SSR/CSR values match |
| `hydration_mismatch` | Client | Keep DOM structure identical; toggle visibility after mount |
| `invalid_raw_snippet_render` | Client | Return single root element from `createRawSnippet` render |
| `legacy_recursive_reactive_block` | Client | Refactor `$:` to separate `$derived` reads from `$effect` writes |
| `lifecycle_double_unmount` | Client | Track mount state; guard `unmount()` calls |
| `ownership_invalid_binding` | Client | Forward `bind:` through the entire prop chain |
| `ownership_invalid_mutation` | Client | Use `$bindable()` or callback props for child→parent updates |
| `select_multiple_invalid_value` | Client | Pass array (or `null`/`undefined`) to `<select multiple>` |
| `state_proxy_equality_mismatch` | Client | Compare proxies with proxies, or use `$state.snapshot` |
| `state_proxy_unmount` | Client | Don't wrap `mount()` result in `$state`; use `$state.raw` if needed |
| `svelte_boundary_reset_noop` | Client | Don't reuse `reset`; re-render boundary instead |
| `transition_slide_display` | Client | Use `block`/`flex`/`grid` display for `slide` transition |
| `dynamic_void_element_content` | Client | Don't pass children to void elements in `<svelte:element>` |
| `state_snapshot_uncloneable` | Client | Exclude DOM elements, functions, and circular refs from snapshot |

---

## Usage Instructions

When a Svelte runtime warning appears in your console:

1. **Identify the warning code** from the console message (e.g., `derived_inert`).
2. **Look up the warning** in the quick reference table or section headers below.
3. **Copy the ✅ corrected example** (JavaScript or TypeScript) that matches your codebase.
4. **Apply the fix** and verify the warning disappears.
5. **If the warning persists**, check for secondary occurrences (e.g., a `$derived` hidden inside a helper function called from an `$effect`).

---

## Client Warnings

---

### `assignment_value_stale`

**Warning:**
```
Assignment to `%property%` property (%location%) will evaluate to the right-hand side,
not the value of `%property%` following the assignment. This may result in unexpected behaviour.
```

**Root Cause:** When using nullish coalescing assignment (`??=`) on a `$state` property, the right-hand
side (`[]`) is pushed to before the proxy replaces it, so the pushed value is lost.

**❌ Problematic:**

```svelte
<!-- JavaScript -->
<script>
    let object = $state({ array: null });

    function add() {
        (object.array ??= []).push(object.array.length);
    }
</script>

<button onclick={add}>add</button>
<p>items: {JSON.stringify(object.array)}</p>
```

```svelte
<!-- TypeScript -->
<script lang="ts">
    let object = $state<{ array: number[] | null }>({ array: null });

    function add() {
        (object.array ??= []).push(object.array.length);
    }
</script>

<button onclick={add}>add</button>
<p>items: {JSON.stringify(object.array)}</p>
```

**✅ Corrected:**

```svelte
<!-- JavaScript -->
<script>
    let object = $state({ array: null });

    function add() {
        object.array ??= [];
        object.array.push(object.array.length);
    }
</script>

<button onclick={add}>add</button>
<p>items: {JSON.stringify(object.array)}</p>
```

```svelte
<!-- TypeScript -->
<script lang="ts">
    let object = $state<{ array: number[] | null }>({ array: null });

    function add() {
        object.array ??= [];
        object.array.push(object.array.length);
    }
</script>

<button onclick={add}>add</button>
<p>items: {JSON.stringify(object.array)}</p>
```

---

### `await_reactivity_loss`

**Warning:**
```
Detected reactivity loss when reading `%name%`. This happens when state is read
in an async function after an earlier `await`.
```

**Root Cause:** State read after an `await` inside a function is not tracked by Svelte's reactivity system.

**❌ Problematic:**

```svelte
<!-- JavaScript -->
<script>
    let a = $state(Promise.resolve(1));
    let b = $state(2);

    async function sum() {
        return await a + b; // `b` is read after `await a`, not tracked
    }

    let total = $derived(await sum());
</script>

<p>total: {total}</p>
```

**✅ Corrected:**

```svelte
<!-- JavaScript -->
<script>
    let a = $state(Promise.resolve(1));
    let b = $state(2);

    async function sum(a, b) {
        return await a + b;
    }

    let total = $derived(await sum(a, b));
</script>

<p>total: {total}</p>
```

```svelte
<!-- TypeScript -->
<script lang="ts">
    let a = $state<Promise<number>>(Promise.resolve(1));
    let b = $state<number>(2);

    async function sum(a: Promise<number>, b: number): Promise<number> {
        return await a + b;
    }

    let total = $derived(await sum(a, b));
</script>

<p>total: {total}</p>
```

---

### `await_waterfall`

**Warning:**
```
An async derived, `%name%` (%location%) was not read immediately after it resolved.
This often indicates an unnecessary waterfall, which can slow down your app.
```

**Root Cause:** Sequential `$derived(await ...)` calls block each other even when independent.

**❌ Problematic:**

```svelte
<!-- JavaScript -->
<script>
    async function one() { return 1; }
    async function two() { return 2; }

    let a = $derived(await one());
    let b = $derived(await two()); // waits for `a` to resolve first
</script>

<p>{a} + {b} = {a + b}</p>
```

**✅ Corrected:**

```svelte
<!-- JavaScript -->
<script>
    async function one() { return 1; }
    async function two() { return 2; }

    let aPromise = $derived(one());
    let bPromise = $derived(two());

    let a = $derived(await aPromise);
    let b = $derived(await bPromise);
</script>

<p>{a} + {b} = {a + b}</p>
```

```svelte
<!-- TypeScript -->
<script lang="ts">
    async function one(): Promise<number> { return 1; }
    async function two(): Promise<number> { return 2; }

    let aPromise = $derived<Promise<number>>(one());
    let bPromise = $derived<Promise<number>>(two());

    let a = $derived(await aPromise);
    let b = $derived(await bPromise);
</script>

<p>{a} + {b} = {a + b}</p>
```

---

### `binding_property_non_reactive`

**Warning:**
```
`%binding%` is binding to a non-reactive property
```

**Root Cause:** Using `bind:` on a property not declared with `$state` or `$bindable`.

**❌ Problematic:**

```svelte
<!-- Parent.svelte — JavaScript -->
<script>
    import Child from './Child.svelte';
    let value = 'hello'; // not reactive
</script>

<Child bind:value />
```

**✅ Corrected:**

```svelte
<!-- Parent.svelte — JavaScript -->
<script>
    import Child from './Child.svelte';
    let value = $state('hello'); // reactive
</script>

<Child bind:value />
```

```svelte
<!-- Parent.svelte — TypeScript -->
<script lang="ts">
    import Child from './Child.svelte';
    let value = $state<string>('hello'); // reactive
</script>

<Child bind:value />
```

---

### `console_log_state`

**Warning:**
```
Your `console.%method%` contained `$state` proxies. Consider using `$inspect(...)`
or `$state.snapshot(...)` instead.
```

**Root Cause:** `console.log` of a `$state` proxy logs the proxy object, not its current value.

**❌ Problematic:**

```svelte
<!-- JavaScript -->
<script>
    let user = $state({ name: 'Alice', age: 30 });

    function handleClick() {
        console.log(user); // logs proxy, not value
    }
</script>
```

**✅ Corrected (Option A — `$inspect` for continuous logging):**

```svelte
<!-- JavaScript -->
<script>
    let user = $state({ name: 'Alice', age: 30 });

    $inspect(user); // logs whenever user changes
</script>
```

**✅ Corrected (Option B — `$state.snapshot` for one-off logging):**

```svelte
<!-- JavaScript -->
<script>
    let user = $state({ name: 'Alice', age: 30 });

    function handleClick() {
        console.log($state.snapshot(user)); // logs plain object
    }
</script>
```

```svelte
<!-- TypeScript -->
<script lang="ts">
    interface User { name: string; age: number; }
    let user = $state<User>({ name: 'Alice', age: 30 });

    function handleClick() {
        console.log($state.snapshot(user));
    }
</script>
```

---

### `derived_inert`

**Warning:**
```
Reading a derived belonging to a now-destroyed effect may result in stale values.
```

**Root Cause:** A `$derived` created inside an `$effect` stops updating when the effect is destroyed
(e.g., on re-run). Create it at the top level or inside `$effect.root`.

**❌ Problematic:**

```svelte
<!-- JavaScript -->
<script>
    let count = $state(0);

    $effect(() => {
        const doubled = $derived(count * 2); // destroyed on every re-run
        console.log(doubled);
    });
</script>
```

**✅ Corrected (Option A — top-level `$derived`):**

```svelte
<!-- JavaScript -->
<script>
    let count = $state(0);
    const doubled = $derived(count * 2); // lives for component lifetime

    $effect(() => {
        console.log(doubled);
    });
</script>

<button onclick={() => count++}>count: {count}</button>
<p>doubled: {doubled}</p>
```

**✅ Corrected (Option B — `$effect.root` for dynamic contexts):**

```svelte
<!-- TypeScript -->
<script lang="ts">
    let count = $state<number>(0);
    let cleanup = $state<(() => void) | null>(null);

    function startTracking(): void {
        cleanup = $effect.root(() => {
            const doubled = $derived<number>(count * 2);
            $effect(() => {
                console.log('doubled:', doubled);
            });
        });
    }

    function stopTracking(): void {
        cleanup?.();
        cleanup = null;
    }
</script>

<button onclick={() => count++}>count: {count}</button>
<button onclick={startTracking}>Start</button>
<button onclick={stopTracking}>Stop</button>
```

---

### `event_handler_invalid`

**Warning:**
```
%handler% should be a function. Did you mean to %suggestion%?
```

**Root Cause:** An event handler was passed a non-function value (e.g., the result of calling a function
instead of the function reference itself).

**❌ Problematic:**

```svelte
<!-- ❌ Calls the function immediately, passes undefined as handler -->
<button onclick={handleClick()}>Click</button>
```

**✅ Corrected:**

```svelte
<!-- ✅ Passes the function reference -->
<button onclick={handleClick}>Click</button>
```

---

### `hydratable_missing_but_expected`

**Warning:**
```
Expected to find a hydratable with key `%key%` during hydration, but did not.
```

**Root Cause:** A hydratable was rendered on the client but not on the server, forcing blocking
synchronous execution during hydration.

**✅ Corrected:**

Ensure the hydratable is rendered on both server and client:

```svelte
<!-- JavaScript — SvelteKit -->
<script>
    import { hydratable } from 'svelte';

    let data = $state(hydratable('foo', fetchData));

    async function fetchData() {
        const res = await fetch('/api/data');
        return res.json();
    }
</script>

{#await data then value}
    <p>{value}</p>
{/await}
```

```svelte
<!-- TypeScript — SvelteKit -->
<script lang="ts">
    import { hydratable } from 'svelte';

    interface Data { message: string; }

    let data = $state(hydratable('foo', fetchData));

    async function fetchData(): Promise<Data> {
        const res = await fetch('/api/data');
        return res.json();
    }
</script>

{#await data then value}
    <p>{value.message}</p>
{/await}
```

---

### `hydration_attribute_changed`

**Warning:**
```
The `%attribute%` attribute on `%html%` changed its value between server and client renders.
The client value, `%value%`, will be ignored in favour of the server value.
```

**Root Cause:** Attributes like `src` differ between SSR and CSR. Svelte keeps the server value to avoid refetching.

**✅ Corrected (force update after mount):**

```svelte
<!-- JavaScript — SvelteKit -->
<script>
    let { src } = $props();

    if (typeof window !== 'undefined') {
        const initial = src;
        src = undefined;
        $effect(() => { src = initial; });
    }
</script>

<img {src} />
```

**Alternative:** Use `<!-- svelte-ignore hydration_attribute_changed -->` if the difference is intentional.

---

### `hydration_html_changed`

**Warning:**
```
The value of an `{@html ...}` block changed between server and client renders.
The client value will be ignored in favour of the server value.
```

**✅ Corrected (force update after mount):**

```svelte
<!-- JavaScript — SvelteKit -->
<script>
    let { markup } = $props();

    if (typeof window !== 'undefined') {
        const initial = markup;
        markup = undefined;
        $effect(() => { markup = initial; });
    }
</script>

{@html markup}
```

---

### `hydration_mismatch`

**Warning:**
```
Hydration failed because the initial UI does not match what was rendered on the server.
```

**Root Cause:** DOM structure from the server differs from the client (e.g., invalid HTML repaired by
the browser, or conditional rendering differs).

**❌ Problematic:**

```svelte
<!-- Renders different elements on server vs client -->
{#if BROWSER}
    <p>Client only</p>
{:else}
    <div>Server only</div>
{/if}
```

**✅ Corrected:**

```svelte
<!-- JavaScript — SvelteKit -->
<script>
    import { onMount } from 'svelte';

    let mounted = $state(false);
    onMount(() => { mounted = true; });
</script>

<!-- Same structure on both; content toggled after mount -->
<div>
    {#if mounted}
        <p>Client only</p>
    {/if}
</div>
```

---

### `invalid_raw_snippet_render`

**Warning:**
```
The `render` function passed to `createRawSnippet` should return HTML for a single element.
```

**Root Cause:** `createRawSnippet`'s render function returned multiple root elements.

**❌ Problematic:**

```js
render: () => '<span>one</span><span>two</span>' // ❌ multiple roots
```

**✅ Corrected:**

```js
render: () => '<div><span>one</span><span>two</span></div>' // ✅ single root
```

---

### `legacy_recursive_reactive_block`

**Warning:**
```
Detected a migrated `$:` reactive block in `%filename%` that both accesses and updates
the same reactive value. This may cause recursive updates when converted to an `$effect`.
```

**Root Cause:** A legacy `$:` block reads and writes the same reactive value, causing an infinite loop
when converted to `$effect`.

**✅ Corrected:**

```svelte
<!-- Svelte 5 runes — JavaScript -->
<script>
    let count = $state(0);

    $effect(() => {
        console.log(count); // read only
    });

    function increment() {
        count += 1; // write only
    }
</script>

<button onclick={increment}>count: {count}</button>
```

---

### `lifecycle_double_unmount`

**Warning:**
```
Tried to unmount a component that was not mounted.
```

**Root Cause:** `unmount()` was called on a component that was already unmounted.

**✅ Corrected:**

```javascript
// JavaScript
let app = mount(MyComponent, { target });
let isMounted = true;

function safeUnmount() {
    if (isMounted) {
        unmount(app);
        isMounted = false;
    }
}
```

```typescript
// TypeScript
let app = mount(MyComponent, { target });
let isMounted = true;

function safeUnmount(): void {
    if (isMounted) {
        unmount(app);
        isMounted = false;
    }
}
```

---

### `ownership_invalid_binding`

**Warning:**
```
%parent% passed property `%prop%` to %child% with `bind:`, but its parent component %owner%
did not declare `%prop%` as a binding.
```

**Root Cause:** A `bind:` chain is broken — a middle component receives a prop but doesn't forward it as `$bindable`.

**❌ Problematic:**

```svelte
<!-- Parent.svelte — receives bind: but doesn't forward it -->
<script>
    let { value } = $props(); // ❌ not bindable
</script>

<Child bind:value />
```

**✅ Corrected:**

```svelte
<!-- Parent.svelte — JavaScript -->
<script>
    import Child from './Child.svelte';
    let { value = $bindable() } = $props(); // ✅ bindable
</script>

<Child bind:value />
```

```svelte
<!-- Parent.svelte — TypeScript -->
<script lang="ts">
    import Child from './Child.svelte';
    let { value = $bindable<string>() } = $props();
</script>

<Child bind:value />
```

---

### `ownership_invalid_mutation`

**Warning:**
```
Mutating unbound props (`%name%`, at %location%) is strongly discouraged.
Consider using `bind:%prop%={...}` in %parent% (or using a callback) instead.
```

**Root Cause:** A child component mutates a prop it doesn't own, without explicit `$bindable` permission.

**✅ Corrected (Option A — `$bindable`):**

```svelte
<!-- App.svelte — JavaScript -->
<script>
    import Child from './Child.svelte';
    let person = $state({ name: 'Florida', surname: 'Man' });
</script>

<Child bind:person />
```

```svelte
<!-- Child.svelte — JavaScript -->
<script>
    let { person = $bindable() } = $props();
</script>

<input bind:value={person.name}>
<input bind:value={person.surname}>
```

**✅ Corrected (Option B — callback props):**

```svelte
<!-- App.svelte — JavaScript -->
<script>
    import Child from './Child.svelte';
    let person = $state({ name: 'Florida', surname: 'Man' });

    function updatePerson(updates) {
        Object.assign(person, updates);
    }
</script>

<Child {person} onupdate={updatePerson} />
```

```svelte
<!-- Child.svelte — TypeScript -->
<script lang="ts">
    interface Person { name: string; surname: string; }
    let { person, onupdate }: { person: Person; onupdate: (u: Partial<Person>) => void } = $props();
</script>

<input bind:value={person.name}
    oninput={(e) => onupdate({ name: e.currentTarget.value })}>
<input bind:value={person.surname}
    oninput={(e) => onupdate({ surname: e.currentTarget.value })}>
```

---

### `select_multiple_invalid_value`

**Warning:**
```
The `value` property of a `<select multiple>` element should be an array,
but it received a non-array value.
```

**❌ Problematic:**

```svelte
<script>
    let selected = $state('apple'); // ❌ string, not array
</script>
```

**✅ Corrected:**

```svelte
<!-- JavaScript -->
<script>
    let selected = $state(['apple']); // ✅ array
    const fruits = ['apple', 'banana', 'cherry'];
</script>

<select multiple bind:value={selected}>
    {#each fruits as fruit}
        <option value={fruit}>{fruit}</option>
    {/each}
</select>
```

```svelte
<!-- TypeScript -->
<script lang="ts">
    let selected = $state<string[]>(['apple']); // ✅ array
    const fruits: string[] = ['apple', 'banana', 'cherry'];
</script>

<select multiple bind:value={selected}>
    {#each fruits as fruit}
        <option value={fruit}>{fruit}</option>
    {/each}
</select>
```

**Alternative:** Use `null` or `undefined` to keep the existing selection unchanged.

---

### `state_proxy_equality_mismatch`

**Warning:**
```
Reactive `$state(...)` proxies and the values they proxy have different identities.
Because of this, comparisons with `%operator%` will produce unexpected results.
```

**Root Cause:** Comparing a `$state` proxy with its original non-proxied value using `===` always returns `false`.

**✅ Corrected:**

```svelte
<!-- TypeScript -->
<script lang="ts">
    interface Foo { foo: string; }
    let value = $state<Foo>({ foo: 'bar' });

    // Compare snapshots for equality checks
    console.log(JSON.stringify($state.snapshot(value)) === JSON.stringify({ foo: 'bar' })); // true
</script>
```

**Note:** Use `$state.raw(...)` if you don't need a proxy and want normal identity semantics.

---

### `state_proxy_unmount`

**Warning:**
```
Tried to unmount a state proxy, rather than a component.
```

**Root Cause:** `unmount()` was called on a `$state`-wrapped component instance.

**❌ Problematic:**

```js
let component = $state(mount(MyComponent, { target })); // ❌ wrapped in $state
unmount(component); // ❌
```

**✅ Corrected:**

```js
let component = mount(MyComponent, { target }); // ✅ no $state
unmount(component); // ✅
```

**If reactivity is needed, use `$state.raw`:**

```ts
let component = $state.raw(mount(MyComponent, { target })); // ✅ no proxy
unmount(component); // ✅
```

---

### `svelte_boundary_reset_noop`

**Warning:**
```
A `<svelte:boundary>` `reset` function only resets the boundary the first time it is called.
```

**Root Cause:** The `reset` function from `<svelte:boundary>` was stored and called more than once.

**✅ Corrected:**

```svelte
<!-- JavaScript -->
<script>
    let errorState = $state(null);
</script>

{#if errorState}
    <p>Error occurred. <button onclick={() => errorState = null}>Retry</button></p>
{:else}
    <svelte:boundary onerror={(e) => errorState = e}>
        <Contents />
    </svelte:boundary>
{/if}
```

```svelte
<!-- TypeScript -->
<script lang="ts">
    let errorState = $state<Error | null>(null);
</script>

{#if errorState}
    <p>Error occurred. <button onclick={() => errorState = null}>Retry</button></p>
{:else}
    <svelte:boundary onerror={(e: Error) => errorState = e}>
        <Contents />
    </svelte:boundary>
{/if}
```

---

### `transition_slide_display`

**Warning:**
```
The `slide` transition does not work correctly for elements with `display: %value%`.
```

**Root Cause:** `slide` animates `height`, which requires `block`, `flex`, or `grid`. It fails on
`inline`, `table`, and `contents`.

**❌ Problematic:**

```svelte
<!-- span is display: inline by default -->
<span transition:slide>Hello world</span>
```

**✅ Corrected:**

```svelte
<!-- JavaScript -->
<script>
    import { slide } from 'svelte/transition';
    let visible = $state(true);
</script>

{#if visible}
    <div transition:slide>Hello world</div> <!-- ✅ block display -->
{/if}
```

Or force the display:

```svelte
<span transition:slide style="display: block;">Hello world</span>
```

---

### `dynamic_void_element_content`

**Warning:**
```
`<svelte:element this="%tag%">` is a void element — it cannot have content.
```

**Root Cause:** `<svelte:element this="input">` (or other void elements like `<img>`, `<br>`) was given children.

**❌ Problematic:**

```svelte
<svelte:element this="input">Some content</svelte:element>
```

**✅ Corrected:**

```svelte
<svelte:element this="input" />
```

Or use a non-void element:

```svelte
<script>
    let tag = 'div';
</script>

<svelte:element this={tag}>Some content</svelte:element>
```

---

### `state_snapshot_uncloneable`

**Warning:**
```
Value cannot be cloned with `$state.snapshot` — the original value was returned.
```

**Root Cause:** `$state.snapshot` tried to clone an object containing uncloneable values (DOM elements,
functions, circular references).

**❌ Problematic:**

```svelte
<script>
    const object = $state({
        property: 'cloneable',
        window: window // ❌ uncloneable
    });

    const snapshot = $state.snapshot(object); // warning emitted
</script>
```

**✅ Corrected:**

```svelte
<!-- TypeScript -->
<script lang="ts">
    interface AppState {
        property: string;
        window: Window | null;
    }

    const object = $state<AppState>({
        property: 'cloneable',
        window: null // ✅ exclude uncloneable values
    });

    const snapshot = $state.snapshot(object); // no warning
</script>
```

**Or filter uncloneable values before snapshotting:**

```typescript
function safeSnapshot<T extends Record<string, unknown>>(obj: T): Partial<T> {
    const cloneable: Partial<T> = {};
    for (const [key, value] of Object.entries(obj)) {
        if (
            typeof value !== 'function' &&
            !(value instanceof Window) &&
            !(value instanceof HTMLElement)
        ) {
            (cloneable as Record<string, unknown>)[key] = value;
        }
    }
    return cloneable;
}
```

---

## Shared Warnings (Client + Server)

### `binding_property_non_reactive` (Shared)

Same as the client-side warning above — fires in both client and server contexts when a `bind:` targets
a non-reactive property. Fix: declare the bound property with `$state` or `$bindable`.
