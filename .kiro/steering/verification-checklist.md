---
inclusion: manual
---

# Verification Checklist

Використовуй після завершення фічі або перед merge. Адаптовано з AI-DLC methodology.

## 1. Компіляція та якість

- [ ] `cargo fmt --check` — без помилок
- [ ] `cargo clippy --all-targets` — 0 warnings
- [ ] `cargo test --workspace` — всі тести pass
- [ ] `getDiagnostics` на змінених файлах — без errors

## 2. Архітектура

- [ ] Новий код в правильному крейті (core vs gui vs cli)
- [ ] Немає GUI imports в `rustconn-core` або `rustconn-cli`
- [ ] Public API не змінено ненавмисно (якщо змінено — задокументовано)
- [ ] Нові модулі зареєстровані в `mod.rs`

## 3. Безпека

- [ ] Паролі/ключі → `SecretString` (не plain String)
- [ ] Немає secrets в логах/помилках
- [ ] CLI паролі через stdin pipe (не `.arg()`)
- [ ] Timeout на всі vault/credential операції

## 4. i18n

- [ ] Всі user-facing strings в `i18n()` / `i18n_f()`
- [ ] Файл додано в `po/POTFILES.in` (якщо нові i18n strings)
- [ ] `display_name()` значення обгорнуті в `i18n()` at call site

## 5. Тестування

- [ ] Новий код покритий property test або integration test
- [ ] Temp files через `tempfile` crate
- [ ] Тести не використовують `unwrap()`/`expect()` без причини

## 6. Документація

- [ ] CHANGELOG.md оновлено (якщо user-facing зміна)
- [ ] `/// # Errors` секція для нових `Result` функцій
- [ ] Коментарі для неочевидної логіки

## 7. Cleanup

- [ ] Немає `dbg!`, `todo!`, `println!`, `eprintln!`
- [ ] Немає `#[allow(dead_code)]` на новому коді
- [ ] Немає закоментованого коду
- [ ] Немає `.clone()` де можна передати `&T`

## Швидка перевірка (делегуй)

```
Делегуй в rust-quality-check: "Run checks with tests"
```

## Коли НЕ потрібен повний чеклист

- Typo fix / коментар → достатньо fmt + clippy
- Тільки .md / .po файли → не потрібно cargo checks
- Тільки hook/steering зміни → не потрібно нічого
