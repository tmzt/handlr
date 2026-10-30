---
title: The daemon's own LLM use, under one [inference] section
date: 2026-10-27
status: In implementation
covers:
  - "One `[inference]` section for every third-party LLM call flankr makes
    itself, outside the coding agents: the general agent's turns, card
    summaries and spoken summaries"
  - "An explicit master switch, off unless set, and everything else
    automatic: backends discovered from a key, uses on by default, models
    chosen per use, never a Pro model by default"
  - "Named backends (`[inference.backends.<name>]`, with a `kind`) and
    per-use overrides (`[inference.uses.<use>]`)"
  - "Run-time reload of the section, and telling the app what is available
    (`hello` and a `server.capabilities` notification)"
  - "Moving the old `[gemini]` and `[summary]` sections over, with old
    configs still working"
  - "Not covered: new backend kinds (only `gemini`); the coding agents' own
    models"
sources:
  - "Owner (2026-10-27), in order: \"plan to rename the config section to
    general_agent, with an enabled option … and infer_backend (only gemini
    supported for now), this should tell the app general_agent_supported\";
    \"should it just be in Hello or should disabling it at runtime in the
    config work?\"; \"maybe general_agent is the wrong config key, we do want
    to enable/disable all third-party LLM usage (such as gemini) outside of
    the coding agents\"; \"should uses just be under inference, and should it
    be possible to declare different models for different uses?\"; \"why not
    inferences.backends.(backend)? that fits inferences.uses\"; \"I want to
    keep the configuration automatic where possible, but I agree that
    enabling third-party llms should be explicit … we shouldn't enable pro
    by default in any case\"; then \"yes, write it as a draft\""
  - "flankr at `d95ec57`:
    - `src/config.rs`: `GeminiConfig { api_key_file, model }` and
      `SummaryConfig { cards }`;
    - `src/server/companion.rs`: `TurnService`, which builds the Gemini engine
      on first use;
    - `src/server/speech.rs`: `speech.summarize` and card summaries;
    - `src/server/conn.rs`: the per-phone `client.options` `general_agent`;
    - `libflankr_proto::companion::HelloResult`, which has no capabilities
      field;
    - `~/.config/flankr/config.toml`: `[gemini] model = \"gemini-3.8-flash\"`"
related:
  - docs/design/20261024_02_SETTINGS_SCREEN_PLAN.md (the phone's General agent toggle)
  - docs/design/20261024_03_GENERAL_AGENT_CHAT_PLAN.md (the general agent's turns)
  - docs/design/20261023_05_SPOKEN_SUMMARIES_PLAN.md (spoken summaries; card summaries per CHANGES.md)
  - docs/design/20261025_02_EXTRACT_LIBINFER_PLAN.md (libinfer and libinfer-gemini, the clients)
---

# The daemon's own LLM use, under one [inference] section

## 1. Goal

flankr calls Gemini in three places of its own, besides the coding agents in
the panes: the general agent's turns, the points and spoken text on Finished
cards, and spoken summaries for reading aloud. Their settings are scattered
(`[gemini]`, `[summary] cards`). Today they are on whenever a key exists, and
the phone can't tell whether the daemon offers a general agent.

After this plan:
- one `[inference]` section governs all of them, with an explicit switch that
  is off unless set, and sensible automatic defaults behind it;
- the daemon picks up changes to that section while it runs;
- the phone is told which uses are available, and adapts at once.

## 2. Decisions

1. **One master switch, off unless set.** `[inference] enabled` defaults to
   `false`, and a config without the section makes no third-party LLM calls
   at all. Turning third-party models on is the one explicit step.
2. **Everything else automatic.** The smallest config that works is:

   ```toml
   [inference]
   enabled = true
   ```

   - **Backend discovered:** with no `[inference.backends.*]` tables, the
     daemon makes one backend, named `gemini`, of kind `gemini`, when a key is
     available: `.private/gemini_api_key` in the daemon's working folder, or
     `GEMINI_API_KEY`. With no key, no backend exists and nothing is available,
     with the reason logged and sent to the phone.
   - **Uses on:** every use (decision 4) is on unless its table says `enabled
     = false`.
   - **Models chosen per use, never Pro:** each backend kind carries default
     models by tier. For `gemini`, that is a Flash model for `general_agent`
     and Flash-Lite for `card_summaries` and `spoken_summaries`, which run
     often. A Pro model is used only when a backend or use names it in
     `model`. The defaults are constants beside the backend kind in
     `src/inference/` and are logged at start, per use.
3. **Named backends** (`[inference.backends.<name>]`), for when the defaults
   aren't wanted:

   ```toml
   [inference]
   enabled = true
   default = "gemini"             # the backend a use takes when it names none

   [inference.backends.gemini]
   kind = "gemini"                # the client type; only "gemini" for now
   api_key_file = ".private/gemini_api_key"
   model = "gemini-3.8-flash"     # this backend's model for every use, overriding the tier defaults
   ```

   - **Kinds and names:** a backend's `kind` picks its client, and two
     backends of one kind may differ in key or model. `default` names the
     backend uses take. Left out, it is the only backend, or the one named
     `gemini`.
   - **Errors:** an unknown `kind`, a `default` naming no backend, or a
     backend without a usable key is an error for that backend alone. It is
     logged and it makes its uses unavailable, but the daemon still starts.
4. **Uses** (`[inference.uses.<use>]`), each optional:

   ```toml
   [inference.uses.general_agent]
   backend = "gemini"             # optional: else `default`
   model = "gemini-3.8-flash"     # optional: else the backend's model or tier default

   [inference.uses.spoken_summaries]
   enabled = false
   ```

   - **The uses:**
     - `general_agent`: the turns the user starts (voice, chat, the centre
       button);
     - `card_summaries`: points and spoken text on Finished cards, was
       `[summary] cards`;
     - `spoken_summaries`: `speech.summarize` while reading aloud.
   - **New uses:** a use added later is on by default too, and its plan names
     it.
   - **Clients:** each distinct (backend, model) pair gets one client, built
     on first use. Uses on the same pair share it.
5. **Old configs keep working.**
   - A top-level `[gemini]` is read as the discovered `gemini` backend (its
     `model` and `api_key_file`), and also turns `[inference] enabled` on, so
     an existing setup is unchanged.
   - `[summary] cards = false` is read as `[inference.uses.card_summaries]
     enabled = false`.
   - Each is logged once as deprecated, naming its new place.
   - A config that has both old and new sections uses the new one, and logs
     the conflict.
6. **Read again while running.**
   - **The watch:** the daemon checks `config.toml`'s modification time every
     5 s and re-reads only `[inference]` (and the deprecated sections) when
     it changes.
   - **Effect:** a change applies from the next call; a turn or summary
     already running finishes. Clients for backends or models no longer used
     are dropped.
   - **A broken section:** one that no longer parses keeps the last good
     settings, and the error is logged.
   - **Other sections:** a change outside these (socket, link, `[[agents]]`)
     is logged as needing a restart.
   - **Keys:** a key file appearing or disappearing is noticed on the same
     5 s check.
7. **Telling the app.**
   - `libflankr_proto::companion::HelloResult` gains `inference:
     Option<InferenceInfo>`, appended and serde-defaulted:

     ```rust
     pub struct InferenceInfo {
         pub general_agent: bool,
         pub summaries: bool,       // card or spoken summaries available
         pub reason: Option<String>, // why not, when either is false
     }
     ```

     `None`, from an older daemon, means as today: both available.
   - **`server.capabilities`** (daemon → phone, a notification) carries the
     same `InferenceInfo` whenever it changes, so the phone adapts without
     reconnecting.
   - **Versioning:** both are additions, so libflankr_proto gets a patch
     release by the owner's rulings.
   - **Models aren't sent:** the phone doesn't need to know which model runs
     what.
8. **The daemon's behaviour per use.**
   - **`general_agent` unavailable:** turns are refused with a clear error
     (`INFERENCE_UNAVAILABLE`, carrying `reason`).
   - **`card_summaries` unavailable:** cards carry no points or spoken text.
   - **`spoken_summaries` unavailable:** read aloud uses the local rules, as
     it does offline.
   - **The phone's toggle:** its `client.options` `general_agent` stays a
     per-phone preference, within what the daemon allows.
9. **The app.**
   - **Without `general_agent`:** the centre button and the Gemini strip are
     hidden. Settings' *General agent* toggle is drawn dim, and its ⓘ gives
     `reason` and names `[inference]` in the daemon's `config.toml`.
   - **Without `summaries`:** cards show no points, and read aloud says the
     reply as the local rules give it.

## 3. Steps

1. **The config:** the `[inference]` types in `src/config.rs` (master switch,
   `default`, backends with `kind`, uses), the tier defaults for `gemini`,
   reading the deprecated sections, and validation.
   **Done when:** config tests cover:
   - no section, meaning off;
   - `enabled = true` alone, which discovers `gemini` with a key and makes
     nothing without one;
   - each use's tier model (Flash or Flash-Lite, never Pro);
   - per-use and per-backend model overrides;
   - an unknown kind and a missing backend, each making only its uses
     unavailable;
   - old `[gemini]` and `[summary]` read as before, logged once.
2. **The daemon on it:** `TurnService`, card summaries and `speech.summarize`
   take their client from the inference settings per use; refusals per
   decision 8; and the 5 s reload of decision 6.
   **Done when:** daemon tests (Gemini mocked) show:
   - each use calling its own backend and model;
   - two uses on one pair sharing one client;
   - turns refused while `general_agent` is off;
   - a reload turning a use off for the next call while a running turn
     finishes;
   - a broken section keeping the last good settings;
   - a key file appearing making the uses available.
3. **The protocol:** `InferenceInfo`, `HelloResult.inference`,
   `server.capabilities`, `INFERENCE_UNAVAILABLE`; libflankr_proto's patch
   release with its VERSIONS.md entry.
   **Done when:** they round-trip, and a hello without `inference` reads as
   `None`.
4. **The client and app:** decision 9's hiding and dimming, live on
   `server.capabilities`.
   **Done when:** client and app tests show the centre button, the Gemini
   strip and the settings toggle following `InferenceInfo`, including a
   change while connected; and on the S23, editing `[inference] enabled` on
   the Mac hides and shows the centre button within about 5 s.

Each step is one crate, one commit (Rule 3.1). Step 2 needs 1; step 3 needs
nothing; step 4 needs 3.

## 4. Open questions for the owner

1. **Per-use prompts:** should a use name its prompt in
   `ADDITIONAL_PROMPTS.md` (for example `prompt = "spoken_summary"`), or are
   the prompts fixed per use in code, as now?
2. **Per-use limits:** a timeout, a maximum output length, or a daily cap per
   use or backend, for cost control?
3. **Local backends:** a future local backend (libinfer's deferred
   llama.cpp client) isn't third-party. Should the master switch gate only
   third-party backends, for example with a per-backend `third_party = false`
   that `enabled = false` leaves usable?
4. **The name:** `[inference]`, or `[llm]`?

## Amendment — 2026-10-27 00:59 EDT: the open questions answered; the section is [llm]

The owner (2026-10-27): "1. can come later, but they should eventually be
aligned. 2. not now 3. it should just enable non coding agent llms 4. llm".
1. **The section is `[llm]`,** in place of `[inference]`, everywhere in this
   plan: `[llm] enabled`, `[llm] default`, `[llm.backends.<name>]` and
   `[llm.uses.<use>]`. The Rust types move to `src/llm/` (decision 2's
   `src/inference/`), and the protocol's `InferenceInfo` is `LlmInfo`, sent as
   `HelloResult.llm` and in `server.capabilities`. The refusal code is
   `LLM_UNAVAILABLE`.
2. **The master switch covers every LLM flankr uses itself,** third-party or
   local, but never the coding agents in the panes. `[llm] enabled = false`
   (the default) means no general agent turns, no card summaries and no
   spoken summaries from any backend. There is no per-backend
   `third_party` flag. This replaces decision 1's "third-party" wording and
   answers open question 3.
3. **Per-use prompts come later.** The prompts stay fixed per use in code for
   now. A later plan should align each use with a named prompt in
   `ADDITIONAL_PROMPTS.md`, and this plan's use names (`general_agent`,
   `card_summaries`, `spoken_summaries`) are chosen so they can be those
   names. This answers open question 1.
4. **No per-use limits now** (timeouts, output length, caps). This answers
   open question 2.

The plan stays `Draft` until the owner approves it.

## Amendment — 2026-10-27 01:00 EDT: approved, to be built now

The owner approved the plan, with its amendment above, on 2026-10-27
("approve and build"). The status becomes `In implementation` with step 1's
commit.

## Amendment — 2026-10-27 01:08 EDT: implementation has started; the document is now immutable

Step 1 (the config) is the first implementing commit. From here the plan
changes only on the owner's explicit instruction; departures go to
`CHANGES.md` (Rule 1.7).

## Amendment — 2026-10-30 13:54 EDT: models chosen by tier and age from the key's model list; approved

On the owner's instruction (Rule 1.5), 2026-10-30: "update our plan for
LLM and approve and start the work", after libinfer's Gemini model choice
plan (libinfer `docs/design/20261030_01_GEMINI_MODEL_CHOICE_PLAN.md`,
libinfer-gemini 0.1.1). The daemon's summaries have never worked: the
Flash-Lite default `gemini-3.8-flash-lite` is not served to the key (336
failed requests, 2026-10-27 to 2026-10-30). This replaces decision 2's
"Models chosen per use" for the `gemini` kind, and the constants
`GENERAL_AGENT_MODEL` and `SUMMARY_MODEL` in `src/llm/gemini.rs`:

1. **Defaults by rule, not by name.** `general_agent` and `planning`:
   `flash:newest`; `card_summaries` and `spoken_summaries`:
   `flash-lite:oldest` (libinfer-gemini's `ModelChoice::Rule`). Still
   never Pro by default.
2. **`model` in a backend or a use** takes the text form
   (`flash-lite:oldest`, `flash:newest`, `pro:newest`) or an exact name,
   read with `ModelChoice::parse`. A `pro:` rule counts as naming Pro.
3. **Resolved once at start.** For each `gemini` backend with a key, the
   daemon fetches `ModelCatalog::fetch` once, with a 10 s timeout, before
   its uses are made; each use's choice is resolved against it with
   `ModelCatalog::resolve`, and the name it gives is the model for the
   run. When the fetch fails, the choices are resolved with
   `resolve_offline` (a rule gives `DEFAULT_MODEL`) and one warning names
   the backend and the reason, never the key.
4. **Logged at start, per use:** the choice, the model it resolved to,
   and how (`[llm] spoken_summaries: flash-lite:oldest is <model>
   (matched)`), replacing the current per-use model line.
5. **Not re-resolved** during the run: a model retired while the daemon
   runs fails its requests until the next start, as now.
6. **libinfer-gemini 0.1.1** is taken in `Cargo.toml` (a path
   dependency, as now) with `Cargo.lock`.

**Steps:**

1. **Resolution.** `src/llm/`: the tier defaults become `ModelChoice`
   rules; `model` strings are parsed; the catalog is fetched and each
   use resolved at start; the start log as item 4. **Done when:** unit
   tests cover the defaults per use, a config `model` in both forms per
   backend and per use, resolution over a `ModelCatalog::from_names` list
   (summaries get the oldest stable Flash-Lite, the general agent the
   `-latest` alias or the newest Flash), and the offline fallback, with no
   network call in tests and no key read.
2. **The daemon on it.** `TurnService`, card summaries and
   `speech.summarize` use the resolved names. **Done when:** `cargo test`
   passes and a start of the test daemon (not the real one) logs one
   resolved line per use.

**Approved** by the owner (2026-10-30, the quotation above), to be built
now in a worktree.
