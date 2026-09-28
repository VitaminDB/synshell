#!/usr/bin/env bash
# Озвучка реплик «Эльвиры» (английский) → assets/voice/<id>.wav (24 кГц моно 16 бит).
# Синтез: synaptix speak (VoxCPM2, 48 кГц). Первой генерируется опорная фраза
# (ref), остальные клонируют её тембр через --reference — один голос на всё.
# Постобработка: обрезка лишнего хвоста (trim_voice.py), loudnorm, 24 кГц.
#
# Usage: tools/gen_voice.sh [id...]      (без аргументов — все; существующие пропускаются)
#        SEED=42 tools/gen_voice.sh ref  (подобрать голос: удалить raw/ref.wav и перегенерировать)
set -euo pipefail

DIR="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$DIR/assets/voice"
RAW="$OUT/raw"
S="${SYNAPTIX:-$HOME/Projects/2027/synaptix/target/release/synaptix}"
M="${MODELS:-$HOME/Storage/syn_models}"
SEED="${SEED:-7}"
mkdir -p "$OUT" "$RAW"

# id|text  (ref — опорная фраза для клонирования, в игру не идёт)
LINES=(
"ref|Welcome back! Let's play, and learn some English together."
"welcome|Welcome back!"
"nice|Nice!"
"good_one|Good one!"
"beautiful|Beautiful!"
"wonderful|Wonderful!"
"excellent|Excellent!"
"amazing|Amazing!"
"incredible|Incredible!"
"magnificent|Magnificent!"
"on_fire|You're on fire!"
"unstoppable|Unstoppable!"
"you_beautiful|You're beautiful!"
"perfect|Perfect!"
"flawless|Flawless!"
"new_record|New record!"
"game_over|Game over. Try again!"
)

want=("$@")
for spec in "${LINES[@]}"; do
    IFS='|' read -r id text <<<"$spec"
    if [[ ${#want[@]} -gt 0 ]] && ! printf '%s\n' "${want[@]}" | grep -qx "$id"; then
        continue
    fi
    raw="$RAW/$id.wav"
    if [[ ! -f "$raw" ]]; then
        echo "== $id: «$text»"
        args=(speak "$M/voxcpm2.syn" "$text" -o "$raw" --device cuda --seed "$SEED")
        [[ "$id" != ref && -f "$RAW/ref.wav" ]] && args+=(--reference "$RAW/ref.wav")
        "$S" "${args[@]}"
    fi
    [[ "$id" == ref ]] && continue
    # VoxCPM2 дописывает к короткой фразе лишнее — оставить только её начало
    # (tools/trim_voice.py), затем нормализация громкости и 24 кГц
    python3 "$DIR/tools/trim_voice.py" "$raw" "$RAW/$id.trim.wav" "$text"
    ffmpeg -y -loglevel error -i "$RAW/$id.trim.wav" \
        -af "apad=pad_dur=0.08,loudnorm=I=-16:TP=-1.5:LRA=7" \
        -ar 24000 -ac 1 -sample_fmt s16 "$OUT/$id.wav"
done
echo "готово: $(ls "$OUT"/*.wav | wc -l) клипов в $OUT"
