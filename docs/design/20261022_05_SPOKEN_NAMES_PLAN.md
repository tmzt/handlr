---
title: Naming agents aloud and on screen
date: 2026-10-22
status: Draft
covers:
  - What handlr calls an agent when it speaks or shows it. That is a
    ladder from the agent's name to its project and its tab, and never
    herdr's pane id
  - Reading herdr's tabs (their number and label), and the project rule
    for workspaces that share a label
  - Spoken forms, including spelling out codes that do not read as English
  - Configuration under `[naming]`: spelling, letter names and aliases
  - Vocabulary for the companion's STT, and addressing an agent by its tab
  - Not covered: how a client renders or speaks a name (plans 20261022_02
    and 20261022_04)
sources:
  - Owner's discussion (2026-10-22). A pane id such as `pZ` means nothing
    to the user. Tabs carry the user's own short names, made short to fit
    a phone's tab bar, with a number prefix only when one was typed.
    Usually one Claude instance runs per project directory, though others
    may do it differently. Short codes without vowels are spelled out by
    their letter names, not the military alphabet. That part should be
    configurable. Naming gets its own plan, because it is not the core of
    the protocol plan.
  - The herdr snapshot on 2026-10-22. It has a top-level `tabs` list with
    `tab_id`, `workspace_id`, `number` and `label`, and panes point at
    their tab by `tab_id`. All 11 Claude panes were in workspace `w2`,
    labelled `cc-highbay`. Their tab labels were `17-hr`, `16-stt`, `-ad`,
    `-mb`, `-bb`, `ctrls`, `-sigsp`, `-sigcc`, `5-kp`, `2` and `1`. The two
    libmorph panes differed only by tab.
  - handlr at `d94abe5`: `notify::agent_identities` and `speakable_name`;
    `core::agents::project_for` and `collect_panes`, which reads tabs only
    when they are nested under workspaces; `core::agents::resolve_pane_id`.
  - Plan 20261022_02's amendments of 17:52 and 17:55 EDT, which this plan
    replaces.
related:
  - docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md (the methods whose
    targets this resolves)
  - docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md (uses these names
    and this vocabulary)
  - docs/design/20261022_04_COMPANION_APP_PLAN.md (speaks and shows them)
---

# Naming agents aloud and on screen

## 1. Goal

When handlr asks "Approve sending continue to …?", announces that an agent
finished, or lists agents, it uses a name you would use. That means
"Claude" when there is one, "the handlr agent" when the project tells them
apart, and "Claude in libmorph, tab em bee" only when it has to. It never
uses `pZ`. The same names, and your tab labels, are what the phone listens
for, so "send continue to aitch ar" reaches the agent in tab `17-hr`.

## 2. Decisions

1. **Where naming lives.** In `core::naming` in the daemon, moved from
   `notify.rs`. The daemon computes every name. Clients show and speak what
   they are given, and have no naming rules of their own.

2. **Tabs.** `core::agents` reads tabs from the snapshot's top-level `tabs`
   list, and still from tabs nested under workspaces where a herdr nests
   them. `AgentRef` gains `tab_label: Option<String>` and `tab_number:
   Option<u32>`, and `common::AgentInfo` gains the same two optional fields.
   Adding them is backwards-compatible on the wire.

3. **The project.** The project is the workspace label, unless more than
   one of the agents being named shares that label. In that case it is the
   basename of the pane's cwd. With one agent per project directory, this
   names each one by its project.

4. **A tab's spoken form.** The label, without a leading number and
   without leading or trailing `-`, with any remaining `-` read as a space.
   - `17-hr` → `hr`
   - `-mb` → `mb`
   - `5-kp` → `kp`
   - `ctrls` → `ctrls`

   A label that is only a number, or is empty, is `tab <number>`. Each word
   then goes through decision 5.

5. **Spelling.**
   - **Which words.** Each word of a spoken name (agent, project, or tab) is
     checked on its own. It is spelled out letter by letter when it is at
     most `spell_max_len` letters (default 6) and contains none of `vowels`
     (default `aeiouy`), because it does not read as English. Digits are
     left as they are.
   - **Examples.**
     - `hr` → "aitch ar"
     - `mb` → "em bee"
     - `kp` → "kay pee"
     - `stt` → "ess tee tee"
     - `ctrls` → "see tee ar el ess"
     - `ad`, `bubble`, `sigsp` and `libmorph` are read as written.
   - **Letter names.** Letters are spelled by their English names, written
     so the TTS says the name and not a sound. These are not the military
     alphabet:

     | a | b | c | d | e | f | g | h | i | j | k | l | m |
     |---|---|---|---|---|---|---|---|---|---|---|---|---|
     | ay | bee | see | dee | ee | ef | jee | aitch | eye | jay | kay | el | em |

     | n | o | p | q | r | s | t | u | v | w | x | y | z |
     |---|---|---|---|---|---|---|---|---|---|---|---|---|
     | en | oh | pee | cue | ar | ess | tee | you | vee | double you | ex | why | zee |

6. **The ladder.** A name has two forms. The *spoken* form uses spoken
   forms and spelling. The *shown* form uses the label as written, such as
   "Claude in libmorph, tab -mb". Each rung is used only when the earlier
   ones are not unique among the current agents:

   1. The agent: "Claude".
   2. The project: "the handlr agent".
   3. The agent in the project: "Claude in libmorph".
   4. The tab: "Claude in libmorph, tab em bee". This is the tab's spoken
      form, or `tab <number>`.
   5. The tab number, when tab labels collide: "Claude in libmorph, tab 24".

   An agent's name is capitalised for speech, as today.

7. **Pane ids are never spoken or shown.** Everything a client presents
   carries a name. `pane_id` stays in messages only as a handle for later
   requests. These fields carry names:
   - `common::AgentInfo` gains `name` (shown) and `spoken` (spoken);
   - `common::ConfirmRequested` gains `name` and `spoken`;
   - plan 02's `notice` and the `ran` entries of a turn's result use them.

   `handlr ctl` prints names, and `serve`'s `confirm <id>` lines name the
   agent and its tab.

8. **Configuration**, in `config.toml`:

   ```toml
   [naming]
   spell_max_len = 6          # 0 turns spelling off
   vowels = "aeiouy"

   [naming.letters]           # any letter's name; unlisted letters keep the default
   w = "dub"

   [naming.aliases]           # say and show this instead
   "ctrls" = "controls"
   "17-hr" = "handlr"
   "agy" = "antigravity"
   ```

   An alias keys on a tab label as written, a project, or an agent name,
   matched case-insensitively. Its value replaces that part of both forms,
   and is not spelled. Aliases come before every other rule. The ladder's
   order is not configurable.

9. **Vocabulary.** This is what plan 02's `vocabulary` and
   `vocabulary.changed` carry:
   - agent names, projects (including cwd basenames) and workspace labels;
   - every tab label as written, and its spoken form;
   - every alias value;
   - for each spelled word, its capitals (`HR`, `MB`, `CTRLS`), the form STT
     writes spelled letters in.

10. **Addressing by name.** Wherever a common method takes `pane_id` or
    `target_id` (`agents.read`, `agents.send`, `agents.focus`, `watch.add`
    and `watch.remove`), the value is resolved against the snapshot, in
    this order:
    1. a pane id;
    2. a tab label as written;
    3. an alias value;
    4. a spoken form, or its capitals;
    5. a tab number.

    Matching is case-insensitive. A value that matches more than one pane
    gets `-32002`, and the error lists the candidates by their shown names.
    This extends plan 20261022_01's resolution without changing it for
    pane ids. A companion turn's `run_command` tool resolves the same way.

## 3. Steps

1. **`core::naming` with the defaults.** Decisions 2–7 and 9, with
   `notify::agent_identities` and `speakable_name` replaced by it. The tests
   use the shape of the 2026-10-22 snapshot:
   - each of the 11 Claude panes gets a name with no pane id;
   - the two libmorph panes are named by tab: spoken "em bee" (shown
     `-mb`) and "ad" (shown `-ad`), because `ad` has a vowel and is read as
     written;
   - "hr", "HR" and "17-hr" resolve to `w2:pZ`;
   - an ambiguous label gets the candidates;
   - each spelling example and all 26 letter names;
   - a 7-letter word with no vowel is read as written.

   **Done when:** `cargo test -p handlr core::naming` passes, and
   `HANDLR_SNAPSHOT=<captured snapshot> cargo test --test live_naming --
   --nocapture` prints, for that snapshot, no name containing a pane id.

2. **`[naming]` configuration.** Decision 8.
   **Done when:** tests show that a letter override changes spelling;
   `spell_max_len = 0` turns spelling off; an alias replaces a tab's name
   in both forms, is in the vocabulary, and resolves; and an unknown key
   under `[naming]` is ignored, as elsewhere in `config.toml`.

3. **Names in the daemon.** The `name` and `spoken` fields of decision 7,
   resolution by decision 10 in every method that takes a target, and names
   in `ctl` output and in `serve`'s token lines.
   **Done when:**
   - against the live herdr, `handlr ctl agents.list` shows each agent's
     name;
   - `handlr ctl agents.read '{"pane_id":"hr","lines":5}'` reads the
     handlr pane;
   - an ambiguous target's error lists the candidates by name;
   - `tests/daemon.rs` covers resolution by tab label and by number over
     the mock.

## 4. Open questions

1. Should `y` count as a vowel? It does by default, which keeps words like
   `gym` and `sync` readable. A code such as `-sy` would then be read as a
   word.
2. Should the phone be able to change an alias by voice ("call ctrls
   controls")? This plan keeps aliases in `config.toml` only.
