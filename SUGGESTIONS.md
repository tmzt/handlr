The answers flankr offers for an agent's pending question, read with
libtypedmd (docs/design/20261027_06_SUGGESTION_RULES_PLAN.md). The daemon
applies the rules and sends each question its answers; the companion app
draws them in order.

A `[Rule]` names the words a question must have (`when`) and must not have
(`unless`), and the answers it then `offers`. In `when` and `unless`, groups
separated by `,` must all be present and words separated by `|` within a
group are alternatives; a word is matched whole, ignoring case and the
punctuation around it. A rule without `when` matches every question. The
rules are tried in order and the first that matches gives the answers.

An `[Answer]` has a `label`, a `style` (`approve`, `deny`, `green`, `link`
or `yellow`), and either `sends` (the text sent when tapped) or `starts`
(the text the message card opens with). `command: true` sends `sends` as
one of the agent's allowed commands; `answers: true` means a tap answers the
question. The texts may use the slots `{{ [abc] detected_plan_id }}` ("plan
06", "the draft" or "the plan"), `{{ [abc] agent }}` and
`{{ [abc] project }}`.

A file at `~/.config/flankr/SUGGESTIONS.md` is merged over this one: an
object with the same tag and name replaces it in place, a new rule is tried
before these, and a new answer is added.

# [Rule] plan

- when: approve, plan | draft
- offers: approve, discuss, implement

# [Rule] question

- offers: yes, no

# [Answer] yes

- label: Yes
- style: approve
- sends: yes
- answers: true

# [Answer] no

- label: No
- style: deny
- sends: no
- answers: true

# [Answer] approve

- label: approve
- style: green
- sends: Approve {{ [abc] detected_plan_id }}
- answers: true

# [Answer] discuss

- label: discuss
- style: link
- starts: Discuss {{ [abc] detected_plan_id }}:

# [Answer] implement

- label: implement
- style: yellow
- sends: Approve {{ [abc] detected_plan_id }} and start building it
- answers: true
