# Cursor AI: Developer Quickstart

A no-BS guide to getting productive with Cursor fast.
Based on actual usage.

---

## What Cursor Actually Is

Cursor is a fork of VS Code with AI baked in.
Your extensions, keybindings, and muscle memory carry over.
The difference: an agent that can read your codebase, edit files, and run commands.

---

## Setup (5 minutes)

1. **Download Cursor** from cursor.com (macOS/Linux/Windows)
2. **Sign in**: free tier gives you limited requests, Pro is $20/mo. We have a Cursor account you can sign in with using your webai.com email.
3. **Open your project**: `File > Open Folder` or `cursor .` from terminal

That's it. If you have VS Code settings, Cursor imports them automatically.

---

## The Four Modes

Cursor has four modes you cycle through with `Shift + Tab`:

- **Agent** — Edits files, runs commands, searches your codebase. Default mode. Use this for building things.
- **Plan** — Proposes changes, waits for your approval before touching anything. Use when you want to review before it acts.
- **Debug** — Adds console logging, analyzes output, finds bugs. Use when something breaks and you don't know why.
- **Ask** — Answers questions about your code without editing. Use when you want to understand, not change.

**Agent mode** is where you'll spend 90% of your time.
The others exist so you can downshift when you need more control.

---

## Essential Shortcuts

- `Shift + Tab` — Cycle between modes
- `Cmd + L` — Open/close the chat panel
- `Cmd + N` — Undo Cursor's last changes
- `Cmd + P` — File/symbol search (same as VS Code)
- `Tab` — Accept inline autocompletion
- `Cmd + Shift + J` — Settings

`Cmd + N` is the safety net. If Cursor makes a mess, hit it.

---

## Prompting That Works

Be specific. Cursor is good at following instructions, bad at reading your mind.

**Weak prompts:**
- "Make it look better"
- "Fix the bug"
- "Add some tests"

**Strong prompts:**
- "Center the form vertically and horizontally using flexbox"
- "The login API returns 401 but the token is valid — add logging to the auth middleware to trace the issue"
- "Add unit tests for the `calculate_total` function covering empty cart, single item, and discount scenarios"

Each prompt takes roughly a minute to process.
Cursor reads your project structure before acting, so the first prompt in a session is slower.

---

## Cursor Rules (Project-Level Instructions)

Rules files let you set persistent instructions so you don't repeat yourself every prompt.

**Create one:**
1. `Cmd + Shift + P` > type "New Cursor Rule"
2. Write your preferences in plain English

**Example rules file:**
```
- Use TypeScript strict mode
- Prefer functional components with hooks
- Use Tailwind for styling, no CSS modules
- Write tests with Vitest, not Jest
- Keep components under 150 lines
```

Rules live in `.cursor/rules/` in your project and apply to every prompt.
Commit them to your repo so the whole team gets the same behavior.

---

## Scaffolding a Project (React + Vite Example)

```
npm create vite@latest my-app -- --template react-ts
cd my-app
npm install
npm run dev
```

Open in Cursor, then prompt:

> "This is a React + Vite + TypeScript project. Read the project structure and add a responsive navigation bar with Home, About, and Contact links using Tailwind CSS."

Cursor will install Tailwind if needed, create components, and wire up routing.

---

## Deploying What You Build

```
npm run build
```

This produces a `dist/` folder with static files. Deploy wherever you want:

- **Vercel** — `npx vercel` (zero-config for Next.js/Vite)
- **Netlify** — `npx netlify deploy --prod` (drag-and-drop also works)
- **GitHub Pages** — push to `gh-pages` branch (free for public repos)
- **Cloudflare Pages** — `npx wrangler pages deploy dist` (fast global CDN)
- **Any static host** — upload the `dist/` folder (it's just HTML/CSS/JS)

---

## Things the Marketing Won't Tell You

- **Cursor burns through API quota fast.** Agent mode makes multiple LLM calls per prompt. Watch your usage if you're on free tier.
- **It hallucinates packages.** Always check that `import` statements reference real libraries before running `npm install`.
- **"No coding required" is a trap.** You can vibe-code a prototype, but debugging and extending it requires understanding what was generated. Read the code Cursor writes.
- **Undo is your friend.** `Cmd + N` after a bad generation. Don't try to prompt your way out of a mess — revert and re-prompt with better instructions.
- **Rules files compound.** Start with 3-5 rules and add more as you notice repeated corrections. Too many rules upfront confuse the model.
- **Plan mode before big changes.** If you're about to refactor something significant, switch to Plan mode first. Review the plan, then let it execute. Saves you from reverting a 15-file change.

---

## Using the CLI Instead (Cursor Agent in the Terminal)

Cursor also ships a CLI agent you can run from the terminal without opening the GUI.
Same AI, same codebase awareness, no editor required.

**Install the CLI:**
Inside Cursor: `Cmd + Shift + P` > "Install 'cursor' command"

This adds `cursor` to your PATH. Verify with `cursor --version`.

**Basic usage:**
```
# Start the agent in your project directory
cursor agent

# Or give it a task directly
cursor agent "add input validation to the signup form"
```

The agent runs in your terminal — reads files, writes diffs, runs commands —
same as Agent mode in the GUI but without the visual editor.

**Use the CLI when:**
- You're already in the terminal
- Running it inside a script or CI
- SSH'd into a remote machine
- Quick one-off task, don't want to open an editor
- Chaining with other CLI tools

**Use the GUI when:**
- You want to see inline diffs visually
- Working on frontend/UI where preview matters
- You want drag-and-drop mode switching
- You need the full VS Code extension ecosystem active
- Editing multiple files and want tabs/splits

**Background mode:**
```
cursor agent --background "refactor all API routes to use the new auth middleware"
```

Useful when you want to kick off a longer refactor and keep working on something else.

**Piping context:**
```
# Feed it specific files
cat src/api/routes.ts | cursor agent "find the bug in this file"

# Feed it error output
npm test 2>&1 | cursor agent "fix the failing tests"
```

The CLI accepts stdin, so you can pipe logs, test output, or file contents directly into it.

---

## Cursor vs Claude Code

Both are AI coding agents. Here's when to reach for which:

- **Interface** — Cursor: GUI (VS Code fork) + CLI. Claude Code: terminal only.
- **Best for** — Cursor: visual work, frontend, rapid prototyping. Claude Code: backend, scripting, CI/CD, complex refactors.
- **File editing** — Cursor: inline diffs in editor. Claude Code: apply patches via tool calls.
- **Context** — Cursor: reads open files + project tree. Claude Code: reads everything you point it at.
- **Customization** — Cursor: rules files. Claude Code: CLAUDE.md + skills + hooks.
- **Extensibility** — Cursor: VS Code extensions. Claude Code: MCP servers, plugins, hooks.
- **Model choice** — Cursor: multiple (GPT-4, Claude, etc.). Claude Code: Claude only.

They're complementary. Use Cursor for UI work where you want to see changes live.
Use Claude Code for deep backend work, multi-step automations, and when you want
fine-grained control over the agent's behavior through skills and hooks.

---

## How to Post This as a Slack Canvas

1. Open the Slack channel where you want to share this
2. Click the **+** button in the message composer
3. Select **Canvas**
4. Give it a title: "Cursor AI: Developer Quickstart"
5. Copy everything from "What Cursor Actually Is" onwards and paste it in — Slack will preserve headers, bold, code blocks, and lists automatically
6. Review the formatting — you may need to re-apply code blocks (select text > click `<>` in the toolbar) since paste sometimes loses them
7. Click **Share** in the top right to post the canvas to the channel

**Tips:**
- Pin the canvas message so it doesn't get buried
- Anyone in the channel can find it later via the channel's **Canvases** tab (bookmark bar)
- Canvases are editable — team members can suggest updates directly
