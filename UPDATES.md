# flankr updates

What has changed in the flankr program that runs on your computer: the daemon (`flankr serve`) and the commands you type (`flankr pair`, `flankr prompt`, `flankr session tail`, `flankr ctl`). It covers the settings in `config.toml`, what it sends to your agents and what it refuses, and the files it reads and writes. Changes that only affect the phone app are in the app's own log.

From now on, each change you can notice ships with a version of the flankr program. The program is version 0.1.0 today. A change that adds a feature, or changes something you would notice, raises the middle number (0.2.0). A fix or a small polish raises the last number (0.1.1). Entries from then on are headed with the version and the date.

# History before versioning

These changes came before the program had version numbers.

## 2026-08-31

**Added**
- Voice on the computer: flankr listened to the computer's microphone and spoke its replies aloud, with a voice that downloads on first use. `--tts-voice` and `--tts-speed` chose the voice and its speed.
- Transports: `[[transports]]` in `config.toml` names the herdr connections, `--transport` picks one, and `--remote user@host` reaches herdr over SSH.
- Settings can be kept in a `.env` file.
- Each agent is set in an `[[agents]]` table, which replaces the old `[commands]` table. `allowed_commands` lists the commands an agent may be sent, and `require_confirmation` makes flankr ask you aloud first.
- Before a command goes to an agent, you must say yes out loud. A yes applies to that one command only.
- `allow_arbitrary_commands` lets a spoken instruction that is not on the allow list be typed to the agent, limited to plain words and the punctuation `. , ' ? :`.
- Gemini reads the recent output of the panes that are working or in focus, not every pane.
- The instructions Gemini is given are in `PROMPT.md`, next to the program.
- Gemini's key can come from a service account file in `.private` or from the `GEMINI_API_KEY` setting.

## 2026-09-01

**Added**
- You can watch and unwatch a pane by voice.
- Agent notifications are spoken without talking over you.
- Agents are named in spoken announcements only as precisely as needed.

## 2026-10-22

**Added**
- `flankr serve` is a daemon that the phone app talks to in text. The phone does the listening and speaking.
- `flankr pair` prints a pairing link and its QR code for the phone to scan.
- The phone link listens on 127.0.0.1:7347 by default, set by `[link] listen`. Setting `listen = []` turns it off.
- `flankr ctl` sends one request to the daemon and prints the reply.
- A command can be confirmed on the phone as well as by voice.
- `interrupt_key` for each agent sends a key to stop it when you interrupt; it defaults to `esc`, and `""` turns this off.
- Agents and projects are named by the `[naming]` settings, including spelling out short names and aliases.

**Changed**
- The Gemini key is read from `.private/gemini_api_key`, in place of the service account sign-in.

**Removed**
- The computer no longer listens or speaks. `--tts-voice` and `--tts-speed` are gone, and so is the voice download.

## 2026-10-23

**Added**
- `flankr session tail` prints an agent's session, with `--pane`, `--follow`, `--entries`, `--history`, `--from` and `--to`.
- Gemini can see each agent's session, so it can answer questions about what the agent has done.
- A question an agent ends its turn with, or a finished pane's last question, is shown to you as a question you can answer.
- A phone that has been silent for a minute counts as asleep, and then gets only what needs you.
- Spoken summaries: when you ask for a reply to be read aloud and it is complex or has code in it, Gemini reads a short summary instead.
- `ADDITIONAL_PROMPTS.md` holds the extra prompts flankr uses, and `--prompts` names another file.
- `[urls] repowiki_url` links the `docs/` paths in an agent's replies to pages on repowiki.
- Text you dictate to the focused agent always waits for your confirmation.

**Changed**
- A project is named after its folder by default. `[naming] project_from = "workspace"` keeps herdr's workspace label instead.
- A Gemini key file that holds a service account JSON is refused, with a reason. It must be an API key.

## 2026-10-24

**Added**
- `flankr session tail --outline` shows a session's sections and where each one starts, and `--from` and `--to` read a range of them.

## 2026-10-25

**Added**
- The sessions of the agy agent are read, as the other agents' sessions are.

**Changed**
- The program is now called flankr. Its settings are in `~/.config/flankr/config.toml`, its paired-device keys in `~/.config/flankr/link_key` and `~/.config/flankr/auth.toml`.
- If `~/.config/flankr/config.toml` is missing and an older `~/.config/handlr/config.toml` exists, flankr copies the settings and the link key across on start, and logs that it did.

## 2026-10-26

**Added**
- Text sent to an agent or the general agent has the symbols it does not allow removed. Text with more than three of the symbols that count against it is refused.

**Changed**
- A tool call that exits with 0 shows as a success, not as a failure.

## 2026-10-27

**Added**
- The general agent can list and read the project's files, with `[llm.uses.planning]`. Its `read` patterns say which files, and `agents_prompt = true` adds the project's AGENTS.md.
- The general agent can see the project's recent commits.
- `flankr prompt` prints the general agent's full prompt. It takes `--use` and `--agent`.
- A `[llm]` section sets up the daemon's own use of Gemini: the general agent, card summaries and spoken summaries. Nothing calls a model until `enabled = true`. The file and the key are read again every 5 seconds when they change.
- Suggested answers for an agent's question come from `SUGGESTIONS.md`. `--suggestions` names another file.
- A terminal view of a session on the phone. It is read-only unless `[terminal] input = true`, and then only into agents with `terminal_input = true`, once you arm it on the phone. An arm lasts 60 seconds from the last key.

**Changed**
- Only the first word of a request can be a command, and any path in it must be inside the project.
- A request the general agent types to an agent starts with `via flankr: `, so the agent can see where it came from.
- A plan question is approved only by a reply that says "approve" with "plan" or "draft".
- `[llm]` replaces the `[gemini]` and `[summary]` sections. Those still work until `[llm] enabled` is set.

## 2026-10-28

**Added**
- Shell panes are listed and open read-only in the terminal view. The `shell` entry in `config.toml` sets this; remove it to refuse their terminal.
- `flankr pair` shows six words, a fingerprint and a confirmation code, and you say yes before a phone is let in. It makes one attempt, and `--timeout` sets its time from 1 to 300 seconds.
- A planning plugin lists the project's plans and their amendments. It is off until `[plugins."dev.flankr.plugins.planning"]` sets `enabled = true`.
- While a phone's terminal is open, `[terminal] attach = true` sizes herdr's tab to the phone and moves herdr's focus to that session's pane. A change you make on the desktop is not forced back.

**Changed**
- `HERDR_SOCKET_PATH` is used when neither `--socket` nor `--transport` is given.

## 2026-10-29

**Added**
- Images can be sent into a session. Files and git output can be opened on the phone, read-only and with a size cap.
- When Claude Code asks a question with choices, the phone shows the choices, and your pick is sent back. Messages to that agent wait while the question is open.
- Claude Code's subagents are listed with their agent and can be read.
- Plugins can offer the general agent their tools. `agent_tools = false` offers none, and `"all"` also offers the tools that change something; those run only after you approve them out loud.

## 2026-10-30

**Changed**
- Gemini models are picked by tier and age from the models your key can use. The general agent uses the newest flash model and summaries use the cheapest flash-lite model. The daemon lists them once when it starts and logs what it chose.

## 0.2.0 — 2026-10-30

**Added**
- When you finish speaking to the phone and replies came in meanwhile, the daemon writes one short spoken update on where things stand, in light of what you said, with the new `spoken_catch_up` prompt in `ADDITIONAL_PROMPTS.md`. It runs on the spoken summaries model and gives up after 8 seconds.

**Changed**
- Spoken summaries leave out which files changed, worktrees and branches, log and report entries, and test counts, unless a reply is only about them.

## 0.2.1 — 2026-10-30

**Fixed**
- A reply with a dash, an ellipsis, an arrow or a tick mark is summarized for the phone again. The daemon dropped the phone's request for it without an answer.

## 0.2.2 — 2026-10-30

**Fixed**
- Spoken and card summaries work: the oldest model is now chosen from 3.x up, since older ones can be listed and still refused.

## 0.2.3 — 2026-10-30

**Fixed**
- If Google refuses a model while the daemon runs, it moves to the next allowed one for that tier and age, and tries again, instead of failing every summary until a restart. A model named exactly in the config is never changed.

## 0.3.0 — 2026-10-30

**Added**
- A choice question Claude is showing can be dismissed from the phone. The daemon presses Esc in Claude's question screen, as dismissing it at the Mac does, but only once it sees the question's first page there; otherwise it sends nothing and tells the phone.

## 0.3.1 — 2026-10-30

**Changed**
- Spoken summaries no longer add a question or an offer the reply didn't make; a plan in the reply is said as a statement.
- Each spoken summary and catch-up is written to the log (`speech.summarize: said …`, `speech.catch_up: said …`), so what was heard can be checked.

## 0.4.0 — 2026-10-30

**Added**
- Gemini can ask a coding agent a question with the new `ask_agent` tool, such as "what did we ship today?". The question goes through the same approval as any request typed to an agent (the phone asks "Approve sending your message to Claude?"; nothing is typed until you approve, and an agent with `allow_messages = false` is never asked). Once approved, the daemon waits up to 2 minutes for the agent's answer in its session, gives it to Gemini (redacted, at most 16 KiB) with no tools available, and Gemini says a short summary. If no answer comes in time, Gemini says the agent is still working and its reply will show on its card. Not available when herdr is reached with `--remote`, since the agent's session files are not on this machine.
