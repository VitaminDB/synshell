#!/usr/bin/env bash
# Генерация исходных текстур отделок Люкса → assets/tex/raw/<id>.png.
# Генератор: synaptix imagine, FLUX.2 klein 9B (~17 с на текстуру; Qwen-Image 2.1
# даёт более тёмные картинки с виньеткой — запасной, MODEL=qwen-image-2.1.syn).
# Уже существующие файлы пропускаются; далее tools/build_atlas.py собирает атлас.
#
# Usage: tools/gen_textures.sh [id...]        (без аргументов — все)
#        MODEL=flux.2-klein-9b.syn tools/gen_textures.sh gold
set -euo pipefail

DIR="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$DIR/assets/tex/raw"
S="${SYNAPTIX:-$HOME/Projects/2027/synaptix/target/release/synaptix}"
M="${MODELS:-$HOME/Storage/syn_models}"
MODEL="${MODEL:-flux.2-klein-9b.syn}"
SIZE="${SIZE:-768}"
STEPS="${STEPS:-28}"
QUANT="${QUANT:-}"
mkdir -p "$OUT"

COMMON="macro photograph, top-down, the surface fills the entire frame edge to edge, strong fine detail, even lighting, photorealistic texture, 8k"
NEG="objects, edges, border, frame, text, watermark, logo, people, hands, blurry, flat, plain, uniform, vignette, dark corners"

# id|seed|prompt
SPECS=(
"gold|11|polished yellow gold metal surface, brushed texture with fine scratches, rich warm reflections and bright specular highlights"
"silver|42|linear brushed silver steel metal surface, fine parallel brush lines, cool grey reflections and bright highlights, flat sheet"
"copper|43|brushed copper metal sheet surface, warm orange-red tone, fine parallel scratches, subtle green patina flecks, no reflections of objects"
"bronze|14|antique bronze metal surface, hammered texture with dark patina in the dents and bright worn highlights"
"glass|15|white frosted glass surface, soft translucent highlights, fine sandblasted grain, gentle light refraction, pale neutral grey"
"crystal|16|clear white quartz crystal cluster surface, sharp sparkling facets, bright refracted highlights, colourless"
"velvet|17|light grey velvet fabric, soft plush nap with crushed sheen, folds catching the light, monochrome"
"oak|18|light oak wood plank surface, fine natural grain lines and small knots, warm honey tone, satin varnish"
"walnut|19|dark walnut wood surface, rich brown flowing grain, satin finish with soft highlights"
"marble|20|white carrara marble surface, soft grey veins and subtle crystalline sparkle, polished"
"granite|21|grey granite stone surface, dense black white and pink speckles, polished with subtle reflections"
"obsidian|22|black obsidian volcanic glass surface, glossy, conchoidal ripples and sharp white highlights"
"lava|23|cooling black lava rock crust with glowing orange molten magma cracks, embers, high contrast"
"ice|24|clear blue glacier ice surface, frost crystals, fine internal cracks and trapped air bubbles"
"jade|25|polished green jade stone surface, translucent milky swirls and darker veins, glossy"
"gem|46|dense pave of many small colourless diamonds covering the entire frame, sparkling facets, white refractions, monochrome"
"pearl|27|iridescent mother-of-pearl nacre surface, soft pink mint and lavender shimmer bands, glossy"
"neon|48|dark matte black panel covered with a fine repeating grid of thin bright white glowing neon lines, circuit board pattern, monochrome"
"leather|29|brown full-grain leather surface, fine natural pebble grain, worn highlights, a line of stitching"
"denim|30|blue denim fabric, visible twill weave, worn fade and white threads"
"porcelain|31|glossy white porcelain ceramic surface with fine crackle glaze lines, monochrome"
"candy|32|glossy white hard candy surface with soft swirls and sugar crystal sparkle, monochrome"
"chocolate|53|dark chocolate bar surface with a repeating pattern of small square segments, glossy tempered finish, soft highlights"
"hologram|34|holographic foil surface, rainbow diffraction gradient, metallic shimmer and fine prism lines"
)

want=("$@")
for spec in "${SPECS[@]}"; do
    IFS='|' read -r id seed prompt <<<"$spec"
    if [[ ${#want[@]} -gt 0 ]] && ! printf '%s\n' "${want[@]}" | grep -qx "$id"; then
        continue
    fi
    out="$OUT/$id.png"
    if [[ -f "$out" ]]; then
        echo "== $id: есть, пропуск"
        continue
    fi
    echo "== $id ($MODEL, seed $seed)"
    args=(imagine "$M/$MODEL" "$prompt, $COMMON" -n "$NEG" -o "$out" --device cuda --width "$SIZE" --height "$SIZE" --steps "$STEPS" --seed "$seed")
    [[ -n "$QUANT" ]] && args+=(--quant "$QUANT")
    "$S" "${args[@]}"
done
echo "готово: $(ls "$OUT" | wc -l) файлов в $OUT"
