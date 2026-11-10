# Flow: предложения правил от контрибутора (канал `.sdh/rule-*`)

**Введено в:** v1.23.0 (T-000241)
**Связанные файлы:**
- [src-tauri/src/sync/rule_proposals.rs](../../src-tauri/src/sync/rule_proposals.rs) — wire-артефакты: валидаторы, имена файлов, effective-резолвер, sha256 канонического JSON
- [src-tauri/src/sync/shared.rs](../../src-tauri/src/sync/shared.rs) — транспорт `sync_rule_proposals` (фаза A под `specs_policy_guard`), `publish_pending_receipts` (фаза B), `sweep_rule_publications`, `sweep_acceptances`, очистка при отключении
- [src-tauri/src/sync/skills.rs](../../src-tauri/src/sync/skills.rs) — `classify → proposal_candidate`, `propose_up`, `protected_rules` в `adopt_main_canon`, `deliver_rules_only`
- [src-tauri/src/commands/rule_proposals.rs](../../src-tauri/src/commands/rule_proposals.rs) — машина решений `decide`, `publish_receipt`/`apply_decision`, `withdraw_rule_proposal`, `resolve_rule_acceptance_conflict`, списки
- [src-tauri/src/db/rule_proposals.rs](../../src-tauri/src/db/rule_proposals.rs) — строки предложений/решений, high-water seq, GC-запросы
- [src/lib/components/RuleProposals.svelte](../../src/lib/components/RuleProposals.svelte) — вкладка «Предложения» на экране «Правила»
- Схема: [rule_proposals](../schema/rule_proposals.md), [rule_proposal_seq](../schema/rule_proposal_seq.md), [rule_acceptances](../schema/rule_acceptances.md), [rule_events](../schema/rule_events.md)
- Смежные flow: [contributor-mode.md](./contributor-mode.md) (роли, канон главного), [rules.md](./rules.md) (канон и возвраты)
- Спека (локальная, gitignored): `docs/superpowers/specs/2026-11-07-rule-proposals-contributor.md`

## Суть

Правка скилла у контрибутора раньше либо молча проигрывала канону главного, либо подменяла его (баг F-000044). Теперь она становится **предложением**: применяется у себя сразу, едет к главному файлом на dev-ветке, а решение главного (accept/reject) возвращается **квитанцией** — неизменяемым файлом, который контрибутор накладывает на свою строку.

Транспорт — тот же git на общей dev-ветке; сервер приложению не нужен:

| Кто пишет | Путь | Содержимое |
|---|---|---|
| Контрибутор | `.sdh/rule-proposals/<rule>.<login>.json` | предложение: `format, login, rule, seq, status (open\|withdrawn), text, at` |
| Главный | `.sdh/rule-decisions/<rule>.<login>.<rev>.json` | квитанция: `action (accept\|reject), decided_rev, decided_by, decided_at` |

`rev` — blob-sha канонического JSON версии предложения: и часть имени квитанции, и CAS-токен решения. Каталоги **вне** общего pathspec обычного синка — ими занимается только `sync_rule_proposals`, чтобы ни merge-драйвер, ни импорт их не трогали.

## Контрибутор: от правки до строки

1. Агент контрибутора правит копию и кладёт `<name>.md.approved`. `classify` даёт `proposal_candidate` (не uptake — канон у контрибутора следуемый, своим менять нельзя).
2. `propose_up` в одной транзакции пишет текст в канон установки (`templates`) и создаёт/обновляет строку [rule_proposals](../schema/rule_proposals.md): `local_state = pending_apply`, `seq = max_seq+1` из [rule_proposal_seq](../schema/rule_proposal_seq.md) — пере-предложение продолжает нумерацию, даже после GC старой строки. Событие `proposed` в истории — один раз на новую открытую версию.
3. Следующий `sync_rule_proposals` поднимает строку `applied` и заливает файл на dev.

Тем же путём идёт **отзыв** (`withdraw_rule_proposal`): `status = withdrawn` уезжает как новая версия того же файла — удалением файла отзыв не выражается, потому что dev-ветку видят все участники.

## Эффективная версия предложения

Файловая пара (dev + origin) и локальная строка сливаются в одну «эффективную» версию: больше `seq` побеждает; при равном `seq` терминальный `withdrawn` старше `open`; равные `seq`/`status`/канонический JSON — no-op. CAS-ядро хэшируется без локальных полей `resolved_*` — оверлей квитанции никогда не меняет `rev`.

## Главный: конвейер решения

`decide(repo, rule, proposer_login, expected_rev, action)` — CAS: если effective-версия сместилась с `expected_rev`, отказ `proposal_rev_mismatch`. Иначе строка [rule_acceptances](../schema/rule_acceptances.md) проходит конвейер, и каждый шаг идемпотентен и переживает падение:

```
prepared ──publish──► decided ──canon txn──► applied ──deliver_rules_only──► rendered
    │                    │                     │
    │                    │                  canon ушёл ─► conflicted (ждёт UI: apply / dismiss → superseded)
    │                    │
    └─ receipt на dev: idempotent path, конфликт байт = ошибка, перезаписи нет
```

- **prepared → decided:** квитанция пишется на dev **под `specs_policy_guard`** (тот же путь plumbing-через-worktree/`commit_files_to_branch`, что у общего синка). Файл неизменяем: байты совпали — ok, разошлись — ошибка, а не overwrite.
- **decided → applied:** `expected_rev` перепроверяется, `base_canon_hash` (sha256 канона на момент решения) сравнивается с текущим. Совпал или канон уже равен предложению — применяем; разошёлся — `conflicted`, канон не трогаем. Reject **никогда** не меняет канон и после квитанции сразу становится `rendered` + событие `proposal_rejected`.
- **applied → rendered:** `deliver_rules_only` доставляет новый канон во все цели без прохода сбора.
- **conflicted:** стабильное состояние, требует `resolve_rule_acceptance_conflict`: `apply` — форс-запись `proposal_text` (не файла — snapshot'а) и дальше по конвейеру; `dismiss` — `superseded`, применение отменено, а публикация квитанции продолжается.
- **overtaken:** если на момент решения появилась более новая версия предложения, старая строка помечается `overtaken` — квитанция остаётся где лежала (неизменяемость), новое решение — отдельной строкой.

## Публикация: `pub_state` отдельно от `status`

`pub_state` — жизнь квитанции на origin и **никогда** не откатывает `status`:

- `pending` — ждёт выгрузки. `publish_pending_receipts` в фазе B пушит вместе с общими файлами.
- `published` — **только** когда квитанция виден в `refs/remotes/origin/<dev>` (snapshot после fetch). Локальный «запушили» не считается: свежесть проверяется `sweep_rule_publications` в начале каждого синка и старта.
- `blocked` — пуш окончательно отклонён (защита ветки/отказ remote; **не** non-fast-forward — тот чинится дивергенцией и повтором). Остаётся в очереди и ретраится.
- `overtaken` — см. выше.

Стартовый sweep (`sweep_acceptances`) поднимает незавершённые строки с того шага, где они остановились: `prepared` снова публикует (idempotent), `decided` заново применяет и т.д. — падение между шагами ничего не теряет и не дублирует.

## Списки и свежесть

`list_rule_proposals` и `list_rule_acceptances` перед чтением прогоняют `sweep_repo_rule_channel` (apply `pending_apply` + resume конвейера + подтверждение публикаций), поэтому отдельного Tauri-события `ProposalChanged` нет — экран перечитывает после действий и синка. Чтение role-aware:

- **главный** — каталог effective-версий с **локального dev tip** + свои строки решений (queued/on_dev/on_origin/разрешение);
- **контрибутор** — свои строки + оверлей квитанций с dev (по `decided_rev` к `rev` версии; квитанция старой версии новую не закрывает).

Дев-ветку контрибутор берёт из `shared_import.pinned_dev_branch` в БД — `.sdh/project.json` в его worktree может отсутствовать (`dev_branch_for`).

## Чужая установка того же логина

Файл предложения адресуется логином, а не установкой: вторая установка контрибутора с тем же GitHub-логином получает чужие версии в `pending_apply` и применяет их к своему канону — две машины одного контрибутора остаются в согласии.

## Отключение и GC

- `disable_shared_mode` / detach контрибутора: строки [rule_proposals](../schema/rule_proposals.md), [rule_acceptances](../schema/rule_acceptances.md) и [rule_proposal_seq](../schema/rule_proposal_seq.md) репо удаляются вместе с остальным `.sdh/`-следам.
- GC терминальных строк только при **доминировании remote-версии**: удалить `withdrawn`-строку можно, когда remote не моложе её (по `remote_ver` — `{seq,status,sha}`), чтобы отзыв не воскрес из-под ещё не дошедшего remote.

## События истории

Миграция v36 расширила `rule_events.kind`: `proposed` (главный, раз на новую версию), `proposal_accepted`, `proposal_rejected`, `proposal_withdrawn` — видны на вкладке «История» правила.
