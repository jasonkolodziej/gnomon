# macOS Safari 27.0

The console shows one red herring — a Cloudflare Insights beacon failing to load (`Could not connect to the server`) — but that's just analytics, not the crash cause.

## Step-by-step investigation plan

Since you have the Web Inspector attached, here's how to isolate the crash:

### 1. Check the **Console** tab for errors before the crash
Refresh the page with the Web Inspector open. Look for:
- `TypeError` or `RangeError` loops
- `Maximum call stack size exceeded`
- Any Svelte hydration mismatch warnings

**Key thing to check:** Does the console show any errors *before* the tab goes black and shows "A problem repeatedly occurred"? If yes, the last error before the crash is your culprit.

### 2. Check the **Sources** tab — set breakpoints
In the Web Inspector Sources tab:
- Press `Cmd+Shift+O` and search for `.svelte.js` or your main entry point
- Set a breakpoint at the top of your `+layout.svelte` or root component
- Refresh and step through — see exactly which line kills the process

### 3. Look at **Timelines** for a CPU spike
In the Timelines tab, start recording before refresh. A runaway `$effect` or infinite loop will show as a 100% CPU spike right before the crash.

### 4. The most likely SvelteKit 2.7 + Safari 27 issue

SvelteKit 2.7 introduced **enhanced client-side navigation** with improved preloading. Combined with Svelte 5 runes, there's a known pattern that can crash WebKit:

**Check if you're using `$effect.pre` or `$effect.root` anywhere.** Safari 27's JavaScriptCore has a regression with certain Proxy + `requestAnimationFrame` interactions that Svelte 5's fine-grained reactivity triggers.

### 5. Quick diagnostic test

Add this to your root `+layout.svelte` temporarily:

```svelte
<script>
  // Force disable all effects and transitions for diagnosis
  import { browser } from '$app/environment';
  
  if (browser) {
    // Disable Svelte transitions globally
    window.__svelte = { disable_transitions: true };
  }
</script>
```

Or try disabling **client-side routing** entirely in `svelte.config.js`:

```js
export default {
  kit: {
    browser: {
      router: false // test with this
    }
  }
};
```

If either stops the crash, you've isolated it to transitions or the router.

### 6. Check Safari 27's new "Experimental Features"

Safari 27 likely has new JIT or WASM features enabled by default that previous versions didn't. Go to **Safari → Settings → Advanced → Experimental Features** and try toggling:
- **JIT Compilation** (try disabling)
- **WebGL 2.0** (if your site uses canvas)
- **CSS Typed OM** (if you use custom properties heavily)

### 7. The definitive test

Create a minimal reproduction — a blank SvelteKit 2.7 project with just a single `$state` and `$effect`. Does *that* crash Safari 27? If yes, it's a core Svelte 5 + Safari 27 bug you should report to both WebKit Bugzilla and the Svelte repo.

---

**What I need from you to go deeper:**

1. Does the Web Inspector console show *any* JS errors before the black screen?
2. Does it crash on **first load** or only on **client-side navigation** (clicking links)?
3. Are you using **Svelte 5 runes** (`$state`, `$derived`, `$effect`) or legacy Svelte 4 reactivity?
4. Any **transitions/animations** on initial page load?

Want me to help you set up a targeted breakpoint strategy based on what you find?
