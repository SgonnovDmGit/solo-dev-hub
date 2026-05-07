# B-000017 — Subpixel flicker on secondary monitor at zoom ≥125%

**Status:** known limitation, not solvable in our code (Chromium/WebView2 internal bug)
**First diagnosed:** 2026-05-07
**Reporter symptom:** "В масштабе 125% по экрану особенно справа сполохи идут, как будто перерисовка заметна постоянная"

## Repro

1. Multi-monitor setup with **different OS DPI scale** на мониторах (Win11 → Display settings → Scale per-monitor)
2. Окно приложения на secondary мониторе
3. UI scale ≥125% (Settings → Внешний вид → Масштаб 125%/150%, или Auto на мониторе ≥2500px logical width)

→ Visible flicker / shimmer на правой ~25% экрана (там где `.content` panel: Bugs/Settings/Dashboard и т.п.)

**Не воспроизводится:**
- На initial monitor (любой zoom)
- На secondary monitor при zoom ≤110%
- На ProjectDetail screen (cytoscape-canvas создаёт отдельный composite layer, маскирующий проблему)

## Root cause

**WebView2 mixed-DPI multi-monitor bug** в Chromium DPI/zoom pipeline.

Когда окно перемещается на монитор с другим OS DPI scale, Chromium внутри WebView2 пересчитывает viewport-метрики. При WebView-level zoom ≥125% применённом через `WebView::setZoom()`, subpixel-anti-aliasing positions становятся off-grid на новом DPI → постоянный subpixel-recompute → visible repaint loop на root layer.

### Trace evidence

Performance trace на secondary мониторе при zoom 125%:
```json
{"name":"Paint","clip":[0,0,2560,0,2560,1392,0,1392],"nodeName":"#document"}
{"name":"Paint","clip":[0,0,2560,0,2560,1392,0,1392],"nodeName":"HTML"}
{"name":"UpdateLayoutTree","elementCount":1}
{"name":"TimerInstall","timeout":20,"url":"sveltekit/runtime/client/client.js:1965"}
```

- Paints на полный viewport (`[0,0,2560,1392]`) — single composite layer covering whole window
- Paints на `#document`/`HTML` ноды — root level, not localised to component
- `elementCount:1` — invalidation одного элемента, но проявляется как full-viewport repaint
- SvelteKit preload-on-hover handlers добавляют шум (на каждый pointermove ставится 20ms таймер) — НЕ root cause, но усиливает

## Что мы попробовали (и почему не сработало)

### 1. CSS layer promotion (`transform: translateZ(0)`, `contain: paint`)

Применили на `.content` и `.app` containers ([routes/+page.svelte](../../src/routes/+page.svelte)).

**Ожидание:** subpixel-AA recompute локализуется на promoted layer'ах, root остаётся стабильным.

**Результат:** Layer borders в DevTools показал, что новые слои появились (2-3 оранжевых прямоугольника вместо 1), но Paint events всё равно идут на `#document`/`HTML` root layer. Сполохи не ушли.

**Вывод:** subpixel-recompute happens INSIDE WebView2 ниже уровня композитинг-слоёв. CSS hints на этом уровне игнорируются.

**Откатили.**

### 2. Switch from `WebView::setZoom()` to CSS `zoom`

Заменили `getCurrentWebview().setZoom(scale)` на `document.documentElement.style.zoom = String(scale)` в [ui-scale.ts](../../src/lib/ui-scale.ts).

**Ожидание:** CSS `zoom` (Chromium non-standard) использует другой rendering path — обходит buggy DPI pipeline.

**Результат:** **Регрессия хуже исходной проблемы.** CSS `zoom` масштабирует контент, **но не уменьшает viewport** (в отличие от WebView setZoom). Поэтому:
- При scale 1.25 → body становится 1.25× ширины окна → правый край режется, titlebar buttons (свернуть/закрыть) не видны
- При scale 0.8 → body 0.8× ширины окна → большая пустая область справа и снизу

**Откатили.**

### 3. `transform: scale()` на body — не пробовали

Теоретически возможен, но требует:
- ручного пересчёта `body { width: calc(100% / scale); height: calc(100% / scale); }`
- compensation для `position: fixed` элементов (titlebar, modals)
- проверки что ничто не использует `getBoundingClientRect()` для измерений (cytoscape, DataGrid sort indicators, etc.)

Большой рефакторинг с непредсказуемыми регрессиями. Не стоит того для bug, который проявляется только в одной конкретной multi-monitor конфигурации.

## Применённые улучшения (positive side-fixes, не решающие основную проблему)

### `data-sveltekit-preload-data="off"` в [src/app.html](../../src/app.html)

SvelteKit по умолчанию ставит preload-on-hover handlers — на каждый `pointermove` устанавливается 20ms таймер для preload данных следующего route'а. В Tauri SPA с store-based screen switching у нас нет SvelteKit-роутов между которыми надо предзагружать → handlers были pure dead overhead, генерируя noise в Performance trace.

Не влияет на сполохи (они продолжаются и без preload), но снижает baseline-load.

### Defensive cleanup `style.zoom` в [ui-scale.ts:initUiScale](../../src/lib/ui-scale.ts)

На случай если предыдущая версия кода (или dev-эксперимент с CSS zoom) оставила inline `style.zoom` на documentElement — стрипаем при init. Иначе CSS zoom стэкается с WebView setZoom при miss-cleanup.

## Workaround для пользователя

В Settings → Внешний вид → Масштаб переключиться с **Авто** на **manual <125%** (100% или 110%) или **≥150%**. Сполохи исчезнут на любом мониторе.

**Альтернатива:** выровнять OS DPI scale на обоих мониторах в Win11 Display Settings (одинаковый %) — это убирает mixed-DPI condition, и WebView2 рендерит корректно.

## Re-evaluation triggers

Переоткрывать B-000017 имеет смысл при:

1. **Major WebView2 runtime update** — Microsoft периодически чинит DPI-related issues в Edge/WebView2. Можно проверять changelog WebView2 на упоминания "DPI", "subpixel", "multi-monitor".
2. **Tauri обновление до v2.x с новой webview-абстракцией** — возможно в будущем Tauri экспонирует API для force-композитинга или альтернативного zoom-path.
3. **Пользовательский report что баг исчез сам** — fix Microsoft'а доехал автоматически.
4. **Появление chromium-issue-tracker записи** с известным workaround — стоит периодически проверить `chromium-issue-tracker` на ключи `mixed dpi multi monitor zoom`.

## Связанные ресурсы

- WebView2 release notes: https://learn.microsoft.com/en-us/microsoft-edge/webview2/release-notes/
- Chromium issue tracker: https://issues.chromium.org/
- Tauri webview docs: https://docs.rs/tauri/latest/tauri/webview/index.html
