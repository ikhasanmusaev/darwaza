#!/usr/bin/env bash
# Каталог сборки Rust и место на диске (#210).
#
# Проблема не теоретическая: за несколько дней каталог дорастает до 11 ГБ, и
# раздел встаёт на 100%. За одну сессию 31 августа это трижды обрывало работу —
# падала сборка пакета для Arch и релизная сборка, каждый раз посреди дела и с
# невнятным «No space left on device» из середины компиляции.
#
# Причина в том, что профилей два. `cargo test` собирает `debug`, `tauri build`
# — `release`, и они не знают друг о друге: 6 ГБ плюс 5 ГБ. Мера, принятая
# раньше (`[profile.dev] debug = "line-tables-only"`), уменьшает вес, но роста
# не отменяет.
#
#   scripts/build-space.sh          # сколько занято и сколько свободно
#   scripts/build-space.sh clean    # убрать отладочный профиль
#   scripts/build-space.sh clean-all  # убрать всё, следующая сборка с нуля
#
# Перед релизной сборкой имеет смысл `clean`: отладочный профиль ей не нужен,
# а места он занимает больше неё самой.
set -euo pipefail

cd "$(dirname "$0")/.."

target="${CARGO_TARGET_DIR:-$(sed -n 's/^ *target-dir *= *"\(.*\)"/\1/p' src-tauri/.cargo/config.toml 2>/dev/null)}"
target="${target:-src-tauri/target}"

[ -d "$target" ] || { echo "каталога сборки нет: $target"; exit 0; }

show() {
  echo "Каталог сборки: $target"
  du -sh "$target" 2>/dev/null | awk '{print "  всего:  " $1}'
  for profile in debug release; do
    [ -d "$target/$profile" ] || continue
    du -sh "$target/$profile" 2>/dev/null | awk -v p="$profile" '{printf "  %-8s %s\n", p ":", $1}'
  done
  echo
  df -h "$target" | tail -1 | awk '{print "Свободно на разделе: " $4 " из " $2 " (" $5 " занято)"}'
}

case "${1:-show}" in
  show) show ;;
  clean)
    rm -rf "${target:?}/debug"
    echo "отладочный профиль убран"
    show
    ;;
  clean-all)
    # Пакеты и подписи внутри release/bundle тоже уйдут — они восстановимы
    # сборкой, но если нужны, скопируйте их заранее.
    rm -rf "${target:?}/debug" "${target:?}/release"
    echo "оба профиля убраны, следующая сборка пойдёт с нуля"
    show
    ;;
  *) echo "использование: $0 [show|clean|clean-all]" >&2; exit 2 ;;
esac
