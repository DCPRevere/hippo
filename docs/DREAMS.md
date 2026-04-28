# Hippo is the memory layer that dreams.

The tagline commits hippo to a specific architectural promise: *the memory does work between conversations*. Every feature has to either deliver on that promise or get out of its way. This document organises hippo's feature set by which part of "dreaming" it delivers.

## Architecture: the Dreamer

Hippo's background processing is performed by **the Dreamer** — a single process that acts on the graph between conversations. There is one Dreamer; it does whatever needs doing on each entity it visits. The actions it can take (linking, inferring, reconciling, consolidating) are internal to the Dreamer, not separate processes.

Multiple workers run the Dreamer in parallel on different entities. Concurrency comes from running many Dreamer copies, not from running multiple kinds of Dreamer. This matches the brain analogy: consolidation during sleep is one process with many effects, not four separate subsystems.

The Dreamer doesn't pull from a queue. It **queries the graph** for its next unit of work. The graph itself is the queue. This means:

- Work selection always reflects the current graph state — never stale.
- New facts written by hippo become eligible for processing on the next Dreamer iteration without explicit enqueue.
- Output from one Dream (e.g. a new edge) is naturally available to the next Dream that lands on a related entity.
- No separate queue table to maintain or recover.

Processing one entity may involve many internal actions and many sub-queries — walking neighbours, checking for contradictions, looking at episodic clusters, asking the LLM what's implied. The Dreamer is broad on each visit, not narrow.

A `Dream` is one execution of the Dreamer on one entity. A `DreamReport` is the aggregated summary of dreaming over a window. The user-facing verb `hippo.dream()` runs the Dreamer for a bounded budget, synchronously, for demos and evals; in production it runs continuously.

### The append-only-for-dreams principle

**Dreams don't destroy.** The Dreamer never deletes a fact, never modifies an existing edge. Its only output is new facts and new edges. Every Dream is purely additive.

This is a strong architectural commitment. Three consequences:

1. **Auditable by construction.** Nothing is ever lost during dreaming. Provenance is structural — you can always trace what the graph looked like at any point in time.
2. **Replayable.** Roll the graph back to time T by ignoring all facts written after T. Useful for evals.
3. **The retrieval layer carries more weight.** Since dreaming-discovered contradictions aren't resolved by deletion, retrieval must understand "this fact is superseded by that fact" and rank accordingly.

Dreaming-discovered supersession is modelled as a regular fact with a dedicated relation type (`supersedes`). Retrieval queries for `not exists (supersedes(this_fact, _))` to filter out superseded facts, weighted by the supersession's confidence.

**User and agent operations *can* destroy.** This is the escape valve. When a fact is genuinely wrong (extraction error, source error), `retract(fact_id)` removes it. `correct(old_fact, new_fact)` is a convenience that retracts and remembers in one operation. These are deliberate, visible operations — not Dream operations.

This distinction matters: dreaming is autonomous and additive; retraction is intentional and destructive. The brand promise that hippo doesn't lose true things to forgetting holds, because forgetting is deliberate.

Salience updates are usage statistics, not facts. Allowed to mutate. Doesn't violate the principle.

### The Dreamer is autonomous

The Dreamer either acts or stays silent. It does not flag work for human review.

- If the Dreamer's confidence is high enough to act, it writes the new fact (a link, an inference, a `supersedes`, a consolidation).
- If confidence is below threshold, it does nothing and moves on. The unit becomes eligible again after the revisit window.
- The dream-report describes what the Dreamer *did*, not what it *couldn't decide*. There is no "needs your attention" surface.

Confidence thresholds are configurable per action. Tuning them lower makes the Dreamer more aggressive; higher makes it more conservative. The default is conservative.

### Design decisions

- **Concurrency control.** Each entity carries a `last_visited` timestamp. The pool serialises the `next_unit` → claim handshake (with a tokio mutex) so two workers can't observe the same entity as un-visited; `mark_visited` is called inside that critical section before `process` runs unlocked. Backed by a real schema column on SQLite/Postgres and an in-memory map on the in-memory backend.
- **Selection strategy (current).** Best-first within the recency filter — each Dreamer's `next_unit` returns the first un-visited entity in `list_entities_by_recency` order. Simple, deterministic, exploits new content first. **Selection-temperature sampling** (weighted exploration/exploitation) is a planned refinement; deferred because in practice every entity gets visited once the recency window resets, so order is cosmetic for the per-pass pattern. Will revisit if hot-region pile-up becomes a measurable problem.
- **Termination.** Continuous in production. For `hippo.dream()` manual triggers, bounded by `dreamer_max_units` or `dreamer_max_tokens` (configurable per deployment). The "dream until idempotent" idea remains an open design: detecting "no effect" is non-trivial when the Dreamer is doing many different actions per visit. Provisional plan: budget-bounded passes; revisit when we have more data on real Dreamer behaviour.
- **Revisit policy.** Time-based for now: don't reprocess an entity within the configured revisit window. Version-based invalidation (revisit when an entity changes) is a possible later refinement; deferred until we know if simple time-based is insufficient.
- **Query ownership.** The Dreamer owns its sub-queries internally. Tunable thresholds live in config; query shape lives in code.
- **First-run rate-limiting.** On a fresh database every entity is a candidate. Configurable warm-up budget caps LLM spend during catch-up.
- **Continuous vs bounded as default.** Bounded by default; continuous is opt-in via config. Avoids surprise LLM bills.

### Configurability

Everything is configurable: confidence thresholds for each action, similarity thresholds, prompt templates, revisit windows, worker counts, model choices, token budgets, selection temperature, the per-action enable/disable flags.

Layered config:
1. Compiled defaults — sensible values, no required config to start.
2. TOML file at a known path — where most ops will work.
3. Per-graph DB overrides — multi-tenant tuning without redeploys.
4. Env vars — deploy-time overrides.

Hot-reload is bounded: threshold and budget changes pick up automatically; structural changes (enabling/disabling actions, adding new ones) require restart.

## Tier 1 — The dream loop itself (must-have, defines the product)

These are non-negotiable. If hippo doesn't ship these, the tagline is a lie.

### 1. The Dreamer runs continuously

The Dreamer runs continuously in the background, with multiple workers in parallel. Visible as a scheduled process with status. *"Hippo dreamed for 12 minutes last night, processed 847 memories, found 23 new connections."* The maintenance code exists; productize it as the Dreamer with observable output, parallel execution, and configurable cadence.

### 2. Link discovery — finding unconnected entities that should be connected

When the Dreamer visits an entity, one of its actions is to look for unlinked-but-close entities and ask the LLM whether they should be connected. **Make this the headline action.** Surface it: every morning the user sees "hippo connected these 7 things while you slept" with the new edges and why. Newly-created links are append-only writes — never overwriting existing edges.

### 3. Inference — generating implied facts from existing structure

While visiting an entity, the Dreamer walks 1-hop neighbours and asks the LLM "what does this imply that we haven't recorded?" Generates new edges from existing structure. *"Alice manages Bob. Bob attends the eng standup. Therefore Alice probably attends or oversees eng standups."* Inferred edges are visibly tagged as inferred, with their derivation chain. Append-only — inferences add facts, never modify them.

### 4. Reconciliation — resolving contradictions through new supersession facts

The Dreamer detects conflicting active edges on an entity and writes a `supersedes` fact when source-credibility evidence is strong enough to act. *"Hippo wrote a supersedes between fact_42 and fact_88 last night, weighted by source reliability."* The original facts remain in the graph; retrieval consults the `supersedes` relation to rank them. Below the confidence threshold, the Dreamer stays silent and revisits later. This timing — delayed, evidence-based, append-only — is unusual; Graphiti and Supermemory resolve at write time and mutate state. Hippo's bet is that delayed reconciliation with more context produces better outcomes, and append-only produces a better audit trail.

### 5. Consolidation: episode → pattern

The brain-inspired action. When the Dreamer finds an entity with many episodic facts in a recent time window, it produces a single semantic-profile fact summarising the pattern, *while keeping the episodes queryable*. The consolidated fact has edges back to its sources. By morning, "your 47 interactions with Sarah this week" has become "Sarah is increasingly stressed about the Q2 launch — see episodes." Append-only: the episodes are not modified.

### 6. Reactivation strengthens — salience that's actually wired up

Currently a field with no logic. Wire it up: every retrieval increments salience on the fact and on linked entities. Salience decays slowly without use. Salience influences retrieval ranking. Crucially: salience is for *ranking*, not deletion — facts are never lost. This is the brain-inspired feature that doesn't require explaining "forgetting" to compliance buyers.

## Tier 2 — Dream observability (the user-facing surface)

The dream loop is invisible by default; these features make it the *product*, not just a backend behaviour.

### 7. The dream-report

A rolling summary surfaced in the SDK and any UI: *"In the last 24 hours hippo processed N memories. Wrote M new links. Wrote K supersession facts. Consolidated these patterns: ..."* Make dreaming visible. This is the difference between "a memory layer with a maintenance job" and "the memory layer that dreams."

The dream-report is purely **observational** — it describes what the Dreamer did. It is *not* a list of things needing human review. The Dreamer is autonomous; if confidence wasn't high enough to act, the Dreamer stayed silent and didn't write anything, and the report doesn't mention it. Anything the report describes is a completed action.

Contents: counts (facts visited, links written, supersessions written, patterns consolidated), notable findings (highest-confidence new links, most-superseded facts), cost (tokens, $), time spent.

For developers: a `DreamReport` object queryable via `GET /dreams/last` or `GET /dreams?from=...`, with drill-downs (which entities, which derivation chains). For end users: a periodic in-app summary. For ops: counts and cost as metrics.

### 8. Dream replay / explainability

For any inferred fact, link, or consolidated pattern, show the derivation chain. *"This link was created because facts A, B, C were close in embedding space and the model reasoned X."* Critical for trust — inferred edges that aren't auditable will erode buyer confidence fast.

### 9. Insight notifications

When dreaming surfaces something significant — a strong contradiction, a high-confidence inference, an anomaly — push it as an insight rather than burying it in a log. *"While dreaming, hippo noticed Alice now mentions Acme 5× more than last month."*

### 10. Manual dream triggers

Let the developer call `hippo.dream()` synchronously to force a pass before a specific query, useful for evals and demos. Most of the time it's automatic; sometimes you want it on demand.

### 11. Dream cost controls

LLM-driven background processing is not free. Surface the cost. *"Last night's dream used 12K tokens, $0.04. Set a budget."* Buyers will reject this feature without budget controls.

## Tier 3 — Architecture features that make dreaming credible

These are the substrate that lets the dream loop work well.

### 12. Multi-tier memory (working / consolidated / archived)

Already partially there. Make the tiers semantically meaningful: *working* = recent episodes, *consolidated* = patterns abstracted by dreaming, *archived* = full detail still queryable. Queries default to consolidated; can opt into episodic.

### 13. Confidence + provenance on every fact

Every fact carries: source, extraction confidence, source's historical credibility, and whether it's directly observed vs. inferred during dreaming. Required for the dream-report and for explainability. Hippo already has parts of this; make it complete and surface-able.

### 14. Source-credibility tracking that compounds

Already in code. Lean into it: *"Hippo learns which of your sources to trust. Your CRM has been right 98% of the time; Slack mentions, 71%. Facts get weighted accordingly."*

### 15. Iterative read path with explicit context loops

Already in code. Position it as part of the dream story: *"hippo doesn't just retrieve — it reasons about what's missing and asks for more, just like thinking."* Underrated feature, deserves more emphasis.

### 16. Embedded / WASM runtime

The other distinctive thing. Pair it with dreaming: *"Hippo dreams locally, in your browser or on your device. Your memory never leaves you."* Dreaming + embedded is the combination no competitor can match without a rewrite.

## Tier 4 — Behaviours competitors can't easily copy

Specific design commitments that follow from the dream metaphor and would be architecturally hard for Mem0/Zep/Supermemory to retrofit.

### 17. Cross-session pattern discovery

Don't just process one user's memory in isolation. During dreaming, look for patterns across the user's entire history: "you mention Acme more on Mondays" or "every time you discuss Project X, Bob is mentioned." This is what "dreaming" enables that "ingest-and-retrieve" doesn't. None of the competitors do this.

### 18. Speculative pre-warming

While dreaming, anticipate likely future queries based on recent patterns and pre-compute the relevant subgraphs. *"You've been asking about Q2 planning every Monday morning — by the time you ask tomorrow, the answer is ready."* Like CDN cache warming, but for thoughts.

### 19. Analogy index

During dreaming, extract structural patterns from episodes ("a tense conversation about a missed deadline") and index them separately from topical content. Lets retrieval find structural matches: "this situation has happened before — here's how it played out." This is hard, distinctive, and brain-inspired.

### 20. Active hypothesis generation

For inferred facts, hippo flags them as hypotheses with confidence. Future episodes that confirm or contradict the hypothesis update its confidence. *"Hippo hypothesized Alice manages Bob; this morning, Alice's email confirmed it — confidence raised to 0.95."* Memory that *learns* from evidence over time.

### 21. Working memory across queries within a session

Spreading-activation analogue. When you ask about Alice, Alice and her neighbours are pre-activated for the next query. Different from chat history — this is the agent's *internal* working state. Particularly useful for voice agents and long sessions.

## Tier 5 — Polish that supports the brand

Features that aren't strictly architectural but make "dreams" feel like a coherent product, not a gimmick.

### 22. Dream visualisation

A simple graph view of "what changed last night." Not Datadog-grade observability — a single-page-app view of recent dream activity. The marketing demo writes itself.

### 23. Dream control vocabulary in the SDK

API verbs that match the metaphor. Not just `hippo.maintain()`:

- `hippo.dream()` — run the Dreamer for a bounded budget, return the dream-report.
- `hippo.dream({ action: "link" })` — run only specific actions for a focused pass.
- `hippo.recall(query)` — read facts (alias for ask).
- `hippo.observe(fact)` — write a new fact (alias for remember).
- `hippo.retract(fact_id)` — explicit destructive removal of a fact. Used when a fact is genuinely wrong (extraction error, source error). Distinct from supersession, which is something the Dreamer writes; retraction is something a user/agent does.
- `hippo.correct(old_fact, new_fact)` — convenience for the common case: retract + observe in one operation.

The vocabulary makes the conceptual model click for developers and reinforces the separation between **dreaming (autonomous, additive)** and **explicit user/agent operations (deliberate, destructive)**.

### 24. Configurable dream depth

Let users tune how aggressive dreaming is. *Light* = dedup + contradictions only. *Deep* = inference + analogy + speculative. Different tradeoffs for different use cases.

### 25. Dream privacy guarantees

For embedded/WASM mode: explicit guarantee that dreaming happens locally, no data leaves the device. Pair with the "your memory never leaves you" angle. This is the privacy story Supermemory's hosted SaaS can't tell.

## What this is *not*

A few things to deliberately *not* build, because they fight the brand:

- **Dreaming that destroys.** The Dreamer never deletes or modifies existing facts. Salience decay yes (a usage statistic, not a fact); destructive edits during dreaming, no. Explicit `retract()` exists for genuine errors but is a deliberate user/agent operation, never a Dream operation.
- **Real-time write performance optimisation.** If we commit to dreaming as the differentiator, accept that hippo's write path is slower than Mem0's. That's the trade. Don't compete on write throughput; compete on what hippo *does with* what's written.
- **Generic graph DB features (Cypher, complex query languages).** Not the brand. The dream loop is the interface; the graph is the implementation. Don't expose the graph as the product.
- **One-shot ingestion / "drop a PDF" demos.** Supermemory's home turf. Hippo's value compounds *over time and use*; the demo should be "watch it get smarter over a week," not "drop a doc, ask a question." Different demo, different positioning.
- **Human-in-the-loop dream review.** The Dreamer either acts or stays silent. The dream-report is observational, not a queue of things needing approval. If we ever want a "review queue" surface, it should be a separate tool over the graph (e.g. surfacing low-confidence facts), not a coupling between dreaming and human attention.

## The shortlist if we can only build five

If the team's small and we have to pick, the five that *most distinctively* deliver "dreams" — the ones competitors structurally can't copy:

1. **The Dreamer running continuously** with **linking** + **inference** as its core actions (#1, #2, #3 — the heart of it)
2. **Consolidation: episodes → patterns** (#5 — brain-inspired, no one has this)
3. **Reactivation strengthens / salience-on-use** (#6 — wires up an existing field)
4. **The dream-report** (#7 — makes dreaming visible)
5. **Embedded / WASM runtime** (#16 — privacy + the only deployment shape competitors can't match)

That's a defensible product. Each feature maps to the tagline. Each one is something Mem0/Zep/Supermemory would have to do real architectural work to match. The append-only-for-dreams principle and `retract`/`correct` user operations come along with #1 — they're not a separate item, they're how the Dreamer is designed.

## What this commits us to

Pricing this honestly: the dream-loop architecture is **expensive**. Background LLM processing on user data is real cost. Hippo's pricing model has to either:

- **Pass it through** — usage-based pricing where dreaming is metered.
- **Bound it** — light/deep tiers with different cost ceilings.
- **Eliminate it** — for embedded/WASM users, dreaming runs on their own LLM API key, so hippo's hosting cost is zero.

The third option is the one that pairs with the embedded story and the "your memory never leaves you" angle. It's also the one that makes hippo profitable at small scale, because the user pays the LLM bill directly. Worth designing for from day one.
