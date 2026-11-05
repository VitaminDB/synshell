//! Справка, которая собирается из живой системы: установленные модели службы
//! с их параметрами и ноды графов `synaptix-flow`. Её получают встроенные
//! навыки `autoskill` (models, nodes) и описания инструментов пула
//! `autotools` (media, pipeline): модель читает их перед генерацией, а не
//! угадывает id моделей и поля нод.

use synaptixd::{ModelKind, ParamKind, ParamSpec};

/// Виды моделей, которые создают медиа.
const MEDIA: &[(ModelKind, &str)] = &[(ModelKind::Image, "Картинки"), (ModelKind::Video, "Видео"), (ModelKind::Music, "Музыка и песни"), (ModelKind::Tts, "Озвучка речи")];

/// Что умеет модель сверх параметров — по её id и архитектуре.
fn abilities(id: &str, arch: &str) -> &'static str {
    match () {
        _ if id.contains("fl2va") => "видео со звуком (речь по репликам из описания, шумы, музыка сцены); ключевые кадры first_frame / last_frame — с картинки начать, картинкой закончить; продолжение ролика: first_frame = последний кадр прошлого",
        _ if id.contains("ref2va") => "видео со звуком (речь по репликам из описания, шумы, музыка сцены); референсы refs — картинки персонажей и предметов, голос, музыка (до 12)",
        _ if arch.contains("h3") || id.contains("minimax") => "видео со звуком",
        _ if id.contains("ltx") => "видео со звуком; audio — ролик под звук (липсинк, движение под музыку); source_video — retake отрезка, extend (продолжить), restyle; control_video — повторить движение/контуры; two_stage — выше качество",
        _ if id.contains("yue") => "песни с вокалом: lyrics с метками [verse]/[chorus]; без текста не для инструментала",
        _ if id.contains("acestep") => "инструментал и песни (lyrics — пусто для инструментала); turbo — быстрее",
        _ => "",
    }
}

fn param_line(p: &ParamSpec) -> String {
    let range = match &p.kind {
        ParamKind::Int { min, max, .. } => format!(" {min}..{max}"),
        ParamKind::Float { min, max, .. } => format!(" {min}..{max}"),
        ParamKind::Choice { options } => format!(" {}", options.join("|")),
        _ => String::new(),
    };
    let def = p.default.as_ref().map(|d| format!("={}", d.display(&p.kind))).unwrap_or_default();
    // Подсказка — только у текстовых полей (у чисел её заменяет диапазон).
    let hint = match &p.kind {
        ParamKind::Text { .. } if !p.hint.is_empty() => format!(" ({})", p.hint),
        _ => String::new(),
    };
    // Имена в инструментах media и pipeline: seconds, negative.
    let key = match p.key.as_str() {
        "duration" => "seconds",
        "negative_prompt" => "negative",
        k => k,
    };
    format!("{key}{range}{def}{hint}")
}

/// Установленные модели медиа: какие по умолчанию, что умеют, параметры.
pub fn models() -> String {
    let mut c = match synaptixd::Client::connect() {
        Ok(c) => c,
        Err(e) => return format!("Служба нейросетей недоступна: {e}"),
    };
    let list = c.list().unwrap_or_default();
    let defaults = c.defaults().unwrap_or_default();
    let mut s = String::from(
        "# Нейросети этой системы\nСобрано из службы synaptixd сейчас. Модель для задачи берётся из настроек пользователя (помечена «по умолчанию») — поле model указывай, только если пользователь сам назвал модель. Параметры — имя диапазон=по умолчанию (seconds — длина в секундах, предел одного ролика или трека; media собирает видео длиннее него роликами цепочкой). Seed −1 — случайный.\n",
    );
    for (kind, title) in MEDIA {
        let mine: Vec<_> = list.iter().filter(|m| m.kind == *kind).collect();
        if mine.is_empty() {
            continue;
        }
        let def = defaults.iter().find(|d| d.kind == *kind).and_then(|d| d.resolved.clone().or(d.model.clone()));
        s.push_str(&format!("\n## {title}\n"));
        for m in mine {
            let mark = if def.as_deref() == Some(m.id.as_str()) { ", по умолчанию" } else { "" };
            let runnable = if m.runnable { "" } else { ", пока не исполняется службой" };
            s.push_str(&format!("- {} — {}{mark}{runnable}.", m.id, m.title));
            let can = abilities(&m.id, &m.arch);
            if !can.is_empty() {
                s.push_str(&format!(" Умеет: {can}."));
            }
            if !m.description.is_empty() {
                s.push_str(&format!(" {}", m.description.trim()));
            }
            if !m.schema.is_empty() {
                s.push_str(&format!("\n  Параметры: {}.", m.schema.iter().filter(|p| p.key != "seed").map(param_line).collect::<Vec<_>>().join(", ")));
            }
            s.push('\n');
        }
    }
    s
}

/// Синтаксис графа и все ноды, которые можно ставить в граф.
pub fn nodes() -> String {
    use synaptix_flow::schema::{FieldType, PortType};
    let ty = |t: PortType| match t {
        PortType::Text => "текст",
        PortType::Number => "число",
        PortType::Image => "картинка",
        PortType::Audio => "звук",
        PortType::Video => "видео",
        PortType::Media => "медиафайл",
        PortType::List => "список",
        _ => "значение",
    };
    let mut s = String::from(
        "# Графы сценариев (инструмент pipeline)\n\
Граф — объект JSON: {\"params\": {имя: {\"type\": \"text|number|file|files\", \"description\": …, \"default\": …}}, \"nodes\": [{\"id\": \"…\", \"op\": \"…\", поля…}], \"output\": [\"id\", …]}.\n\
- Поле узла — значение или ссылка: {{param}} — параметр, {{id.text}} — текст узла, {{id.file}} — его файл, {{id.files}} — все файлы (после each), {{id.lines}} — текст узла построчно (список).\n\
- \"each\": \"{{список}}\" — выполнить узел для каждого элемента, внутри полей {{item}}; результаты — список.\n\
- Длинный ролик (длиннее одного ролика модели): узел video с \"each\": \"{{сцены.lines}}\" и \"chain\": true — каждый следующий ролик начинается с последнего кадра предыдущего (одна непрерывная сцена), потом concat.\n\
- Звук видео (речь героев, шумы, музыку сцены) делает видеомодель: реплики пиши в prompt сцены в кавычках. Отдельную музыку и mux — только если просили.\n\
- Поле model не указывай — модели из настроек пользователя (см. навык models).\n\
- output — id узлов-результатов; они покажутся пользователю.\n\
\n## Ноды (op)\n",
    );
    for n in synaptix_flow::ops::specs().filter(|n| !n.stage) {
        s.push_str(&format!("- {} — {}: {}.", n.kind, n.title, n.description));
        if !n.inputs.is_empty() {
            s.push_str(&format!(" Входы: {}.", n.inputs.iter().map(|p| format!("{} ({})", p.name, ty(p.ty))).collect::<Vec<_>>().join(", ")));
        }
        let fields: Vec<String> = n
            .fields
            .iter()
            .filter(|f| n.input(f.name).is_none())
            .map(|f| {
                let choice = match f.ty {
                    FieldType::Choice(o) => format!(": {}", o.iter().filter(|x| !x.is_empty()).copied().collect::<Vec<_>>().join("|")),
                    _ => String::new(),
                };
                let def = if f.default.is_empty() || f.default == "0" || f.default == "false" { String::new() } else { format!(" = {}", f.default) };
                format!("{} ({}{choice}){def}", f.name, f.label)
            })
            .collect();
        if !fields.is_empty() {
            s.push_str(&format!(" Поля: {}.", fields.join(", ")));
        }
        if !n.outputs.is_empty() {
            s.push_str(&format!(" Выход: {}.", n.outputs.iter().map(|p| format!("{} ({})", p.name, ty(p.ty))).collect::<Vec<_>>().join(", ")));
        }
        s.push('\n');
    }
    s
}

/// Встроенные навыки `autoskill`: имя, когда загружать, текст.
pub fn skills() -> [(&'static str, &'static str, fn() -> String); 2] {
    [
        ("models", "какие нейросети установлены, какая по умолчанию, что умеет каждая и её параметры — перед генерацией медиа и на вопросы о моделях", models),
        ("nodes", "синтаксис графов pipeline и все ноды с входами и полями — перед тем как собрать свой граф", nodes),
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn nodes_list_ops_and_chain() {
        let s = super::nodes();
        assert!(s.contains("- video — "));
        assert!(s.contains("chain"));
        assert!(s.contains("- concat — "));
    }
}
