//! Фазы цикла: отрезки текущего цикла и карточка с описанием фазы — что происходит,
//! самочувствие, что поможет, шанс зачатия. Открывается нажатием на полосу фаз,
//! кнопку фазы под ней или метку текущей фазы в шапке.

use syngui::prelude::*;
use syngui::GestureDetector;
use syngui::StyleValue;

use super::{boxed, icon, icon_button, W};
use crate::date::{self, Day};
use crate::icons;
use crate::model::{Phase, Phases};
use crate::state::St;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseKey {
    Period,
    Follicular,
    Fertile,
    Luteal,
    Late,
}

impl PhaseKey {
    pub const CYCLE: [PhaseKey; 4] = [PhaseKey::Period, PhaseKey::Follicular, PhaseKey::Fertile, PhaseKey::Luteal];

    pub fn of(p: Phase) -> Self {
        match p {
            Phase::Period => PhaseKey::Period,
            Phase::Follicular => PhaseKey::Follicular,
            Phase::Fertile | Phase::Ovulation => PhaseKey::Fertile,
            Phase::Luteal => PhaseKey::Luteal,
            Phase::Late => PhaseKey::Late,
        }
    }

    /// Класс цвета в стилях.
    pub fn tone(self) -> &'static str {
        match self {
            PhaseKey::Period => "period",
            PhaseKey::Follicular => "folli",
            PhaseKey::Fertile => "fert",
            PhaseKey::Luteal => "lute",
            PhaseKey::Late => "late",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            PhaseKey::Period => icons::DROP,
            PhaseKey::Follicular => icons::LIGHT,
            PhaseKey::Fertile => icons::FLOWER,
            PhaseKey::Luteal => icons::DARK,
            PhaseKey::Late => icons::TIME,
        }
    }

    /// Короткое имя (кнопки фаз).
    pub fn short(self) -> String {
        match self {
            PhaseKey::Period => t!("Менструация"),
            PhaseKey::Follicular => t!("Фолликулярная"),
            PhaseKey::Fertile => t!("Овуляторная"),
            PhaseKey::Luteal => t!("Лютеиновая"),
            PhaseKey::Late => t!("Задержка"),
        }
    }

    /// Подпись вкладки — помещается в четверть узкой шапки.
    pub fn tab(self) -> String {
        match self {
            PhaseKey::Period => t!("Месячные"),
            PhaseKey::Follicular => t!("Фоллик."),
            PhaseKey::Fertile => t!("Овуляция"),
            PhaseKey::Luteal => t!("Лютеин."),
            PhaseKey::Late => t!("Задержка"),
        }
    }

    pub fn title(self) -> String {
        match self {
            PhaseKey::Period => t!("Менструация"),
            PhaseKey::Follicular => t!("Фолликулярная фаза"),
            PhaseKey::Fertile => t!("Овуляция и фертильное окно"),
            PhaseKey::Luteal => t!("Лютеиновая фаза"),
            PhaseKey::Late => t!("Задержка"),
        }
    }

    fn what(self) -> String {
        match self {
            PhaseKey::Period => t!("Эстроген и прогестерон на минимуме, и слизистая матки, подготовленная в прошлом цикле, обновляется — начинаются месячные. Это первый день нового цикла. Обычно они длятся 3–7 дней."),
            PhaseKey::Follicular => t!("В яичниках созревают фолликулы, один из них становится ведущим. Эстроген растёт, слизистая матки восстанавливается и утолщается. Фаза начинается с месячными и длится до овуляции; на полосе — её часть после месячных."),
            PhaseKey::Fertile => t!("Резкий подъём лютеинизирующего гормона запускает овуляцию: зрелая яйцеклетка выходит из фолликула. Она живёт около суток, а сперматозоиды — до 5 дней, поэтому шанс зачатия есть примерно 6 дней: пять до овуляции и день овуляции."),
            PhaseKey::Luteal => t!("На месте фолликула образуется жёлтое тело и вырабатывает прогестерон — он готовит слизистую матки к беременности. Если её нет, к концу фазы гормоны снижаются и начинается новый цикл. Фаза довольно постоянна: обычно 12–16 дней."),
            PhaseKey::Late => t!("Месячные не начались в ожидаемый день. Задержка в несколько дней бывает из-за стресса, болезни, перелёта, смены веса или нагрузки, а также после отмены гормональной контрацепции."),
        }
    }

    fn feel(self) -> Vec<String> {
        match self {
            PhaseKey::Period => vec![t!("Спазмы внизу живота, тянущая боль в пояснице"), t!("Усталость и меньше энергии, особенно в первые дни"), t!("Возможны головная боль и чувствительность к холоду")],
            PhaseKey::Follicular => vec![t!("Прилив энергии и сил"), t!("Лучше настроение, концентрация и память"), t!("Чище кожа, легче даются нагрузки")],
            PhaseKey::Fertile => vec![t!("Пик энергии и либидо"), t!("Прозрачные тянущиеся выделения, похожие на яичный белок"), t!("У некоторых — лёгкая боль сбоку внизу живота в день овуляции")],
            PhaseKey::Luteal => vec![t!("Спокойнее и тише, хочется уюта"), t!("Ближе к концу — признаки ПМС: чувствительная грудь, вздутие, тяга к сладкому"), t!("Раздражительность и более чуткий сон")],
            PhaseKey::Late => vec![t!("Признаки ПМС могут держаться дольше обычного"), t!("Иногда — тянущие ощущения, как перед месячными")],
        }
    }

    fn tips(self) -> Vec<String> {
        match self {
            PhaseKey::Period => vec![t!("Тепло на живот и отдых без чувства вины"), t!("Больше воды и продуктов с железом: мясо, бобовые, гречка, шпинат"), t!("Мягкое движение — прогулка, растяжка, йога")],
            PhaseKey::Follicular => vec![t!("Хорошее время для интенсивных тренировок"), t!("Планируйте новые дела и важные встречи"), t!("Белок и свежие овощи поддержат рост энергии")],
            PhaseKey::Fertile => vec![t!("Планируете беременность — это лучшие дни"), t!("Не планируете — используйте надёжную контрацепцию: календарь её не заменяет"), t!("Пейте больше воды: выделений и жажды больше")],
            PhaseKey::Luteal => vec![t!("Ложитесь вовремя: прогестерон делает сон чувствительнее"), t!("Меньше соли, сахара и кофеина — меньше отёков и перепадов настроения"), t!("Магний, сложные углеводы и умеренные нагрузки")],
            PhaseKey::Late => vec![t!("Если возможна беременность — сделайте тест"), t!("Задержка больше недели или повторяется — стоит обратиться к гинекологу"), t!("Отметьте первый день, когда месячные начнутся: прогноз подстроится")],
        }
    }

    /// Шанс зачатия: подпись и уровень 0..2.
    fn chance(self) -> (String, u8) {
        match self {
            PhaseKey::Period => (t!("Низкий"), 0),
            PhaseKey::Follicular => (t!("Растёт"), 1),
            PhaseKey::Fertile => (t!("Высокий"), 2),
            PhaseKey::Luteal => (t!("Низкий"), 0),
            PhaseKey::Late => (t!("—"), 0),
        }
    }

    fn hormones(self) -> String {
        match self {
            PhaseKey::Period => t!("Всё на минимуме"),
            PhaseKey::Follicular => t!("Эстроген растёт"),
            PhaseKey::Fertile => t!("Пик ЛГ и эстрогена"),
            PhaseKey::Luteal => t!("Прогестерон высокий"),
            PhaseKey::Late => t!("—"),
        }
    }
}

/// Отрезки цикла по фазам: (фаза, первый день, последний день).
pub fn segments(p: &Phases) -> Vec<(PhaseKey, Day, Day)> {
    let mut v = vec![(PhaseKey::Period, p.start, p.period_end)];
    if p.fertile.0 > p.period_end + 1 {
        v.push((PhaseKey::Follicular, p.period_end + 1, p.fertile.0 - 1));
    }
    v.push((PhaseKey::Fertile, p.fertile.0.max(p.period_end + 1), p.fertile.1.min(p.next - 1)));
    if p.next - 1 > p.fertile.1 {
        v.push((PhaseKey::Luteal, p.fertile.1 + 1, p.next - 1));
    }
    // пустые отрезки (окно внутри месячных при странных отметках) не показываем
    v.retain(|&(_, a, b)| a <= b);
    v
}

/// Фаза, к которой относится день текущего цикла.
pub fn key_at(p: &Phases, d: Day) -> PhaseKey {
    segments(p).into_iter().find(|&(_, a, b)| d >= a && d <= b).map(|(k, _, _)| k).unwrap_or(PhaseKey::Luteal)
}

/// Дни фазы в цикле: «Дни 6–10» и даты.
fn days_line(p: &Phases, k: PhaseKey) -> Option<(String, String, i32)> {
    let (_, a, b) = segments(p).into_iter().find(|s| s.0 == k)?;
    let (da, db) = (a - p.start + 1, b - p.start + 1);
    let days = if a == b { t!("День {n} цикла", n = da) } else { t!("Дни {a}–{b} цикла", a = da, b = db) };
    Some((days, date::range(a, b), b - a + 1))
}

fn bullet(text: String, tone: &str, glyph: Option<&str>) -> W {
    let mark: W = match glyph {
        Some(g) => boxed(icon(g).class(&format!("ic pi-bullet-ic {tone}"))),
        None => boxed(DecoratedBox::new().class("pi-dot-wrap").child(DecoratedBox::new().class(&format!("pi-dot {tone}")))),
    };
    boxed(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Start).child(mark).child(DecoratedBox::new().class("grow").child(Text::new(text).max_lines(5).class("pi-text"))))
}

fn fact(label: String, value: String, extra: Option<W>) -> W {
    let mut col = Column::new().gap(4.0).child(Text::new(label).max_lines(1).class("pi-fact-label")).child(Text::new(value).max_lines(2).class("pi-fact-value"));
    if let Some(e) = extra {
        col = col.child(e);
    }
    boxed(DecoratedBox::new().class("pi-fact grow").child(col))
}

/// Шкала шанса зачатия — три деления.
fn meter(level: u8, tone: &str) -> W {
    let mut row = Row::new().gap(3.0);
    for i in 0..3u8 {
        row = row.child(DecoratedBox::new().class(&if i <= level { format!("pi-meter on {tone}") } else { "pi-meter".to_string() }));
    }
    boxed(row)
}

fn body(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let k = st.phase_info.get();
        let fc = st.fc.get_untracked();
        let today = fc.today;
        let ph = fc.phases(today);
        let cur = fc.phase().map(PhaseKey::of);
        let tone = k.tone();
        let mut col = Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        col = col.child(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(DecoratedBox::new().class("grabber")));

        // заголовок: знак фазы, название, дни и даты
        let (days, dates) = match (k, ph.as_ref().and_then(|p| days_line(p, k))) {
            (PhaseKey::Late, _) => (tn!(fc.late, "{n} день задержки", "{n} дня задержки", "{n} дней задержки"), String::new()),
            (_, Some((d, r, _))) => (d, r),
            _ => (String::new(), String::new()),
        };
        let mut texts = Column::new().gap(3.0).child(Text::new(k.title()).max_lines(2).class("pi-title"));
        if !days.is_empty() {
            let sub = if dates.is_empty() { days } else { format!("{days} · {dates}") };
            texts = texts.child(Text::new(sub).max_lines(1).class("pi-sub"));
        }
        col = col.child(
            Row::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class(&format!("pi-badge {tone}")).child(icon(k.glyph()).class("ic pi-badge-ic")))
                .child(DecoratedBox::new().class("grow").child(texts))
                .child(icon_button(icons::CLOSE, "nav-btn", move || st.phase_open.set(false))),
        );
        if cur == Some(k) {
            if let Some(cd) = fc.cycle_day(today) {
                col = col.child(Row::new().child(DecoratedBox::new().class(&format!("pi-here {tone}")).child(Text::new(t!("Вы здесь · день {n} цикла", n = cd)).max_lines(1).class("pi-here-text"))));
            }
        }

        // переключатель фаз
        if k != PhaseKey::Late {
            let mut sw = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
            for p in PhaseKey::CYCLE {
                let on = p == k;
                sw = sw.child(
                    GestureDetector::new().class("grow").on_click(move || st.phase_info.set(p)).child(
                        DecoratedBox::new().class(&if on { format!("pi-tab on {}", p.tone()) } else { "pi-tab".to_string() }).child(
                            Column::new().gap(5.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class(&format!("pi-tab-dot {}", p.tone()))).child(Text::new(p.tab()).max_lines(1).class(if on { "pi-tab-text on" } else { "pi-tab-text" })),
                        ),
                    ),
                );
            }
            col = col.child(sw);
        }

        // ключевые факты
        let (chance, level) = k.chance();
        let mut facts = Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        if let Some((_, _, n)) = ph.as_ref().and_then(|p| days_line(p, k)).filter(|_| k != PhaseKey::Late) {
            facts = facts.child(fact(t!("Длится"), tn!(n, "{n} день", "{n} дня", "{n} дней"), None));
        }
        facts = facts.child(fact(t!("Зачатие"), chance, if k == PhaseKey::Late { None } else { Some(meter(level, tone)) }));
        facts = facts.child(fact(t!("Гормоны"), k.hormones(), None));
        col = col.child(facts);

        let section = |title: String| Text::new(title).class("pi-section");
        col = col.child(Column::new().gap(8.0).child(section(t!("Что происходит"))).child(Text::new(k.what()).max_lines(12).class("pi-text")));
        let mut feel = Column::new().gap(10.0).child(section(t!("Как можно себя чувствовать")));
        for f in k.feel() {
            feel = feel.child(bullet(f, tone, None));
        }
        col = col.child(feel);
        let mut tips = Column::new().gap(10.0).child(section(t!("Что поможет")));
        for f in k.tips() {
            tips = tips.child(bullet(f, tone, Some(icons::CHECK)));
        }
        col = col.child(tips);
        col = col.child(Text::new(t!("Прогноз по календарю — ориентир, а не диагноз и не способ контрацепции. Цикл у всех разный.")).max_lines(4).class("hint"));
        col = col.child(Button::new(t!("Понятно")).class("btn big primary").on_click(move || st.phase_open.set(false)));
        vec![boxed(col)]
    }))
}

fn panel(st: St, wide: bool) -> W {
    let vp = viewport_size().get_untracked();
    let maxh = (vp.height * 0.9).max(320.0);
    let mut sheet = DecoratedBox::new().class(if wide { "sheet wide" } else { "sheet" });
    if wide {
        sheet = sheet.style("width", StyleValue::px(vp.width.min(560.0)));
    }
    boxed(GestureDetector::new().on_click(|| {}).child(sheet.style("max-height", StyleValue::px(maxh)).child(ScrollView::new().vertical().class("sheet-scroll").child(DecoratedBox::new().class("sheet-inner").child(body(st))))))
}

pub fn layer(st: St, wide: bool) -> W {
    let scrim = Reactive::new(move || -> Vec<W> {
        let shown = st.phase_open.get();
        let vp = viewport_size().get();
        vec![boxed(
            Presence::new(shown, GestureDetector::new().on_click(move || st.phase_open.set(false)).child(DecoratedBox::new().class("scrim").style("width", vp.width).style("height", vp.height)))
                .enter(Motion::fade())
                .exit(Motion::fade())
                .duration_ms(240)
                .initial(false),
        )]
    });
    boxed(Stack::new().fit(StackFit::Expand).child(scrim).child(
        Column::new().main_axis_alignment(if wide { MainAxisAlignment::Center } else { MainAxisAlignment::End }).cross_axis_alignment(if wide { CrossAxisAlignment::Center } else { CrossAxisAlignment::Stretch }).child(Reactive::new(move || -> Vec<W> {
            let shown = st.phase_open.get();
            vec![boxed(
                Presence::new(shown, DecoratedBox::new().child(panel(st, wide)))
                    .enter(if wide { Motion::fade().scale(0.94).slide(0.0, 24.0) } else { Motion::fade().slide(0.0, 260.0) })
                    .exit(if wide { Motion::fade().scale(0.96) } else { Motion::fade().slide(0.0, 260.0) })
                    .duration_ms(340)
                    .exit_duration_ms(200)
                    .initial(false),
            )]
        })),
    ))
}
