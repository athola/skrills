# Tome Research — AI Tool Token Conservation Discourse

> Captured 2026-04-26 during cold-window v0.8.0 polish.
> Source: tome:discourse-scanner agent (HN / Lobsters / Reddit /
> GitHub Discussions) at the request of the user
> (`/attune:mission` continuation, "do deep research online
> using the tome plugin for other projects on github or
> hackernews").

## TL;DR

Community sentiment strongly validates skrills' tier thresholds,
with one nuance: the 20 K Advisory tier is **more** defensible
than the 50 K tier as a "warning" floor, while the 80 % Warning
tier is **under-protective** given that Claude Code's own
auto-compact at ~83.5 % (167 K of 200 K) is widely reported as
triggering data loss. The dominant 2025-2026 narrative is
(a) "Expensively Quadratic" cost curves emerging at 20-27.5 K
tokens, (b) MCP tool-definition bloat eating 55-143 K tokens
before user input, and (c) cloud-injected skills silently
consuming ~6 K tokens per Claude Code session with no opt-out.
Counter-evidence: a vocal minority (e.g., HN's `TZubiri`) argues
cached input tokens are "virtually free" and that quadratic
concerns are overstated for short-lived sessions.

## Theme 1 — Quadratic / Supra-linear Cost Behavior

The canonical reference is **"Expensively Quadratic: the LLM
Agent Cost Curve"** (blog.exe.dev, Feb 2026), which made HN
front page (HN item 47000034). Key operative claims:

- **"In the default settings of the simulator, it only takes
  20,000 tokens to get to the point where cache reads
  dominate."** — directly anchors skrills' 20 K Advisory tier.
- **"By 50,000 tokens, your conversation's costs are probably
  being dominated by cache reads."** — directly anchors
  skrills' 50 K Caution tier.
- **"At the end of the conversation, cache reads are 87 % of
  the total cost. They were half the cost at 27,500 tokens!"**
  — quantifies the cost inflection.

Counter-evidence: HN `TZubiri` argues "cached input tokens are
almost virtually free naturally (unless you hold them for a
loong period of time)." A March 2026 Anthropic pricing change
eliminated the long-context surcharge above 200 K, weakening
the *dollar-cost* argument but leaving the *latency / quality*
argument intact.

## Theme 2 — MCP Overhead and Recommended Ceilings

**Simon Willison, "too many model context protocol servers and
LLM allocations on the dance floor"** (simonwillison.net, Aug
2025), citing Geoffrey Huntley:

- Cursor's *usable* window is ~176 K (200 K minus ~24 K system
  prompt).
- "Adding just the popular GitHub MCP defines 93 additional
  tools and swallows another 55,000 of those valuable tokens."
- Real-world measurement: "Three MCP servers — GitHub,
  Playwright, and an IDE integration — consumed 143,000 of a
  200,000-token context window before an agent read its first
  user message."
- RAG-MCP benchmark: "tool selection accuracy collapsed from
  43 % to under 14 %" with bloated tool sets.
- Anthropic shipped Tool Search GA in Feb 2026 claiming "85 %
  reduction in token usage."

Validates skrills' 50 K Caution tier — single popular MCP gets
you most of the way there.

## Theme 3 — Skill / Agent / Subagent Bloat

**GitHub anthropics/claude-code#39686** (closed as not planned):
"claude.ai Skills and Cowork plugins silently injected into
Claude Code context — no opt-out, ~6 K tokens wasted per
session." Per-source attribution from the issue:

- 43 claude.ai Skills: ~3,950 tokens
- 26 Cowork Plugins: ~2,020 tokens
- Single skill cost examples: `anthropic-skills:loom` 205 t,
  `anthropic-skills:xlsx` 241 t

Direct evidence that **per-source token attribution is a desired
feature the upstream tool does not surface clearly enough**,
validating skrills' Skill / Plugin / MCP / Conversation
breakdown approach.

## Theme 4 — Token Attribution: Who/What Eats Tokens?

The Claude Code `/context` slash command output categories align
with skrills' attribution model: System prompt, System tools,
MCP tools, Custom agents, Memory files, Skills, Messages, Free
space, Autocompact buffer. **claudefa.st** characterizes the
"33 K-45 K Token Problem" — the buffer reserved before the
user's first input, mostly invisible to users.

DEV Community / Threads post (melvynxdev): **"you're losing
33 % of your context window for nothing… disable auto-compact"**
— direct user revolt against opaque overhead.

## Theme 5 — Kill-Switch / Circuit-Breaker Patterns

**GitHub anthropics/claude-code#6123** (CRITICAL): "Auto-compact
triggers at 8-12 % context making Claude Code unusable" — when
the kill-switch fires too early, users are paying $100/mo for
an unusable tool.

**Issue #28728 + #11819 + #46695**: extensive feature requests
for a *configurable* auto-compact threshold; current default
is ~83.5 % (compacts at ~167 K of 200 K). The
`CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` env var was added in response.

Aider's `--map-tokens` (default 1 K, 2 × multiplier when no
files added) is the closest parallel circuit-breaker pattern;
issue #752 documents it not being respected. Cautionary note
for skrills: thresholds *and* enforcement matter.

## Threshold Validation Table

| skrills tier | Community evidence | Verdict |
|---|---|---|
| **20 K Advisory** | exe.dev simulator: cache-read dominance starts here; HN `cs702` confirms | **Strongly supported.** Anchored in real cost-curve data. |
| **50 K Caution** | Willison / Huntley: single GitHub MCP = 55 K. exe.dev: cache-reads dominate by here. | **Strongly supported** as the "MCP-load + early conversation" ceiling. |
| **80 % Warning** | Claude Code auto-compact fires at ~83.5 % with documented data loss. | **Supported but consider 75 %.** Late-2025 refinements moved Anthropic's trigger to 64-75 % to avoid failed compactions. |
| **100 % Kill-switch** | Issue #6123 shows premature kill-switch is worse than late kill-switch. | **Supported, but require user override path.** Never make the kill-switch un-disableable. |

## Quotes Worth Surfacing in skrills Docs

1. **Geoffrey Huntley (via Simon Willison, Aug 2025):**
   "Adding just the popular GitHub MCP defines 93 additional
   tools and swallows another 55,000 of those valuable tokens."
   — Motivate the 50 K Caution tier.
2. **exe.dev, "Expensively Quadratic" (Feb 2026):** "It only
   takes 20,000 tokens to get to the point where cache reads
   dominate… by 50,000 tokens, your conversation's costs are
   probably being dominated by cache reads." — Anchor the 20 K
   and 50 K tiers in one citation.
3. **HN `embedding-shape` (Feb 2026, item 47000034):** "Less
   tokens you can give the LLM with only the absolute
   essentials, the better." — Epigraph for the dashboard's
   philosophy.
4. **anthropics/claude-code#39686 reporter (2025):** "~6,000
   tokens wasted per session… 3 % of total capacity consumed
   by unwanted content." — Motivate per-source attribution as
   a first-class feature.
5. **melvynxdev (Threads, 2025):** "You're losing 33 % of your
   context window for nothing." — Contrarian / user-pain-point
   quote in the kill-switch section.

## Sources

- [Expensively Quadratic: the LLM Agent Cost Curve (blog.exe.dev)](https://blog.exe.dev/expensively-quadratic)
- [HN discussion item 47000034](https://news.ycombinator.com/item?id=47000034)
- [Lobsters: Expensively Quadratic](https://lobste.rs/s/0stawc/expensively_quadratic_llm_agent_cost)
- [Simon Willison: too many MCP servers (Aug 2025)](https://simonwillison.net/2025/Aug/22/too-many-mcps/)
- [Geoffrey Huntley: too many MCPs](https://ghuntley.com/allocations/)
- [anthropics/claude-code#39686 — silent skill injection](https://github.com/anthropics/claude-code/issues/39686)
- [anthropics/claude-code#6123 — auto-compact at 8-12 %](https://github.com/anthropics/claude-code/issues/6123)
- [anthropics/claude-code#28728 — configurable auto-compact](https://github.com/anthropics/claude-code/issues/28728)
- [anthropics/claude-code#46695 — context_threshold setting](https://github.com/anthropics/claude-code/issues/46695)
- [anthropics/claude-code#18159 — false context limit](https://github.com/anthropics/claude-code/issues/18159)
- [Claude Code /context command (jdhodges.com)](https://www.jdhodges.com/blog/claude-code-context-slash-command-token-usage/)
- [Save tokens in Claude Code — wmedia.es](https://wmedia.es/en/tips/claude-code-save-tokens-10-habits)
- [Context Buffer 33K-45K problem (claudefa.st)](https://claudefa.st/blog/guide/mechanics/context-buffer-management)
- [MCP context bloat (Atlassian)](https://www.atlassian.com/blog/developer/mcp-compression-preventing-tool-bloat-in-ai-agents)
- [Your MCP server is eating your context window (Apideck)](https://www.apideck.com/blog/mcp-server-eating-context-window-cli-alternative)
- [Stop Claude Code from lobotomizing itself mid-task](https://ianlpaterson.com/blog/stop-claude-code-from-lobotomizing-itself-mid-task/)
- [Disable auto-compact pro tip (Threads/melvynxdev)](https://www.threads.com/@melvynxdev/post/DS8uN9ciayp/)
- [Context Rot research (Chroma)](https://research.trychroma.com/context-rot)
- [Aider repo-map docs](https://aider.chat/docs/repomap.html)
- [Aider issue #752 — repo-map limit not respected](https://github.com/Aider-AI/aider/issues/752)
- [Cursor context window 2026 (Morph)](https://www.morphllm.com/cursor-context-window)
- [Anthropic API pricing 2026 (Metacto)](https://www.metacto.com/blogs/anthropic-api-pricing-a-full-breakdown-of-costs-and-integration)
