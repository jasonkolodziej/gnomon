# Svelte 5 Runes — Detailed Reference

Comprehensive JS and TypeScript examples for all Svelte 5 runes.

## $state — Reactive State

### Basic Usage

**JavaScript**:

```svelte
<script>
	let count = $state(0);
	let user = $state({ name: 'Alice', age: 30 });
</script>

<button onclick={() => count++}>Clicks: {count}</button>
<button onclick={() => user.age++}>Age: {user.age}</button>
```

**TypeScript**:

```svelte
<script lang="ts">
	type User = { name: string; age: number };

	let count = $state<number>(0);
	let user = $state<User>({ name: 'Alice', age: 30 });
</script>

<button onclick={() => count++}>Clicks: {count}</button>
<button onclick={() => user.age++}>Age: {user.age}</button>
```

### Deep Reactivity

Objects and arrays become deeply reactive proxies. Mutations trigger updates:

**JavaScript**:

```js
let todos = $state([{ done: false, text: 'learn svelte' }]);
todos[0].done = true; // triggers update
todos.push({ done: false, text: 'build app' }); // triggers update
```

**TypeScript**:

```ts
type TodoItem = { done: boolean; text: string };

let todos = $state<TodoItem[]>([{ done: false, text: 'learn svelte' }]);
todos[0].done = true; // triggers update
todos.push({ done: false, text: 'build app' }); // triggers update
```

**Important**: When you destructure reactive state, references are NOT reactive:

```js
let { done, text } = todos[0];
todos[0].done = true; // `done` variable won't update
```

### $state in Classes

**JavaScript**:

```js
class Todo {
    done = $state(false);
    constructor(text) {
        this.text = $state(text);
    }
    reset = () => {
        this.text = '';
        this.done = false;
    };
}
```

**TypeScript**:

```ts
class Todo {
    done = $state(false);
    text: string;

    constructor(text: string) {
        this.text = $state(text);
    }

    reset = () => {
        this.text = '';
        this.done = false;
    };
}
```

### $state.raw — Non-Reactive Objects

Use for large objects where deep reactivity is not needed (performance optimization):

**JavaScript**:

```js
let data = $state.raw({ large: 'dataset' });
data.large = 'new value'; // no effect, must reassign entire object
data = { large: 'new value' }; // this works
```

**TypeScript**:

```ts
let data = $state.raw<{ large: string }>({ large: 'dataset' });
data.large = 'new value'; // no effect, must reassign entire object
data = { large: 'new value' }; // this works
```

---

## $derived — Computed Values

### Basic Usage

**JavaScript**:

```svelte
<script>
	let count = $state(0);
	let doubled = $derived(count * 2);
	let tripled = $derived(count * 3);
</script>

<p>{count} × 2 = {doubled}</p>
<p>{count} × 3 = {tripled}</p>
```

**TypeScript**:

```svelte
<script lang="ts">
	let count = $state<number>(0);
	let doubled = $derived(count * 2);
	let tripled = $derived(count * 3);
</script>

<p>{count} × 2 = {doubled}</p>
<p>{count} × 3 = {tripled}</p>
```

### $derived.by — Complex Logic

**JavaScript**:

```js
let numbers = $state([1, 2, 3]);
let total = $derived.by(() => {
    let sum = 0;
    for (const n of numbers) sum += n;
    return sum;
});
```

**TypeScript**:

```ts
let numbers = $state<number[]>([1, 2, 3]);
let total = $derived.by(() => {
    let sum = 0;
    for (const n of numbers) sum += n;
    return sum;
});
```

### Overriding Deriveds (Svelte 5.25+)

Useful for optimistic UI updates:

**JavaScript**:

```js
let likes = $derived(post.likes);

async function onclick() {
    likes += 1; // optimistic update
    try {
        await likePost();
    } catch {
        likes -= 1; // rollback on error
    }
}
```

**TypeScript**:

```ts
let likes = $derived(post.likes);

async function onclick(): Promise<void> {
    likes += 1; // optimistic update
    try {
        await likePost();
    } catch {
        likes -= 1; // rollback on error
    }
}
```

---

## $effect — Side Effects

### Basic Usage

**JavaScript**:

```svelte
<script>
	let size = $state(50);
	let canvas;

	$effect(() => {
		const ctx = canvas.getContext('2d');
		ctx.clearRect(0, 0, canvas.width, canvas.height);
		// reruns when `size` changes
		ctx.fillRect(0, 0, size, size);
	});
</script>

<canvas bind:this={canvas} width="100" height="100"></canvas>
```

**TypeScript**:

```svelte
<script lang="ts">
	let size = $state<number>(50);
	let canvas = $state<HTMLCanvasElement | null>(null);

	$effect(() => {
		if (!canvas) return;
		const ctx = canvas.getContext('2d');
		if (!ctx) return;
		ctx.clearRect(0, 0, canvas.width, canvas.height);
		ctx.fillRect(0, 0, size, size);
	});
</script>

<canvas bind:this={canvas} width="100" height="100"></canvas>
```

### Teardown / Cleanup

Return a function to clean up when the effect re-runs or the component unmounts:

```js
$effect(() => {
    const interval = setInterval(() => count++, 1000);
    return () => clearInterval(interval); // cleanup
});
```

### Dependency Tracking Rules

Effects automatically track any `$state`/`$derived` read synchronously. Async reads (after `await`) are NOT tracked:

```js
$effect(() => {
    context.fillStyle = color; // tracked
    setTimeout(() => {
        context.fillRect(0, 0, size, size); // size NOT tracked!
    }, 0);
});
```

Conditional dependencies — only depends on values read in the last run:

```js
$effect(() => {
    if (condition) {
        confetti({ colors: [color] }); // only depends on color if condition is true
    }
});
```

### $effect.pre

Runs BEFORE DOM updates. Rare — use for things like preserving scroll position before a re-render.

---

## $props — Component Props

### Basic Usage

```svelte
<!-- Parent.svelte -->
<Child message="hello" count={42} />

<!-- Child.svelte (JavaScript) -->
<script>
	let { message, count = 0 } = $props(); // destructuring with defaults
</script>

<p>{message} - {count}</p>
```

```svelte
<!-- Child.svelte (TypeScript) -->
<script lang="ts">
	interface Props {
		message: string;
		count?: number;
	}

	let { message, count = 0 }: Props = $props();
</script>

<p>{message} - {count}</p>
```

### Special Cases

**Renaming props** (for reserved words or invalid identifiers):

```js
let { super: trouper = 'default' } = $props();
```

**Rest props**:

```js
let { a, b, ...others } = $props();
```

---

## $bindable — Two-Way Binding

### Usage

**JavaScript**:

```svelte
<!-- FancyInput.svelte -->
<script>
	let { value = $bindable(), ...props } = $props();
</script>

<input bind:value {...props} />

<!-- Parent.svelte -->
<script>
	import FancyInput from './FancyInput.svelte';
	let message = $state('hello');
</script>

<FancyInput bind:value={message} />
<p>{message}</p>
```

**TypeScript**:

```svelte
<!-- FancyInput.svelte -->
<script lang="ts">
	type InputProps = {
		value?: string;
		placeholder?: string;
	};

	let { value = $bindable(''), ...props }: InputProps = $props();
</script>

<input bind:value {...props} />

<!-- Parent.svelte -->
<script lang="ts">
	import FancyInput from './FancyInput.svelte';
	let message = $state<string>('hello');
</script>

<FancyInput bind:value={message} />
<p>{message}</p>
```

**Fallback values**:

```js
let { value = $bindable('default value') } = $props();
```

**Use sparingly**: Most components should use callback props instead of `$bindable`.
