//! Модальное окно отчёта покрытия библиотеки шаблонов Fusion (C2):
//! открывается по меню «Шаблоны → Покрытие шаблонов Fusion» или из палитры.

use syngui::prelude::*;
use syngui::widgets::*;

use super::St;
use crate::fusion::report::{self, CoverageReport};

pub fn open(st: St) {
    let rep = report::analyze(&crate::fusion::library(), None);
    st.coverage_report.set(Some(rep));
}

fn close(st: St) {
    st.coverage_report.set(None);
}

pub fn view(st: St) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let Some(rep) = st.coverage_report.get() else { return Vec::new() };
        vec![Box::new(
            Stack::new()
                .fit(StackFit::Expand)
                .child(GestureDetector::new().on_click(move || close(st)).child(DecoratedBox::new().class("fp-scrim")))
                .child(Center::new().child(EventHook::new().on_key_down(move |k, _| {
                    if k == Key::Escape {
                        close(st);
                        return KeyReply::Handled;
                    }
                    KeyReply::Ignore
                }).child(card(st, &rep)))),
        )]
    })
}

fn card(st: St, rep: &CoverageReport) -> impl Widget {
    let head = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new("Покрытие библиотеки шаблонов Fusion").class("fp-title"))
        .child(DecoratedBox::new().class("grow"))
        .child(Button::new("Обновить").class("head-btn").on_click(move || open(st)))
        .child(ToolButton::new("\u{E5CD}").tooltip("Закрыть (Esc)").on_click(move || close(st)));

    let summary = DecoratedBox::new().class("lp-item").child(
        Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(format!("Всего: {}", rep.total)).class("lp-name"))
                    .child(DecoratedBox::new().class("lp-badge lp-badge-on").child(Text::new(format!("100% готовы: {} ({:.1}%)", rep.full, rep.full_pct())).class("lp-badge-text")))
                    .child(DecoratedBox::new().class("lp-badge").child(Text::new(format!("Разобрано: {} ({:.1}%)", rep.parsed, rep.parsed_pct())).class("lp-badge-text")))
                    .child(DecoratedBox::new().class("lp-badge").child(Text::new(format!("Инструменты: {}/{}", rep.supported_tool_types, rep.total_tool_types)).class("lp-badge-text"))),
            )
            .child(Text::new(rep.root.clone()).max_lines(1).elide(Elide::Middle).class("lp-path")),
    );

    let mut body = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch);

    // Разделы
    body = body.child(Text::new("ПО РАЗДЕЛАМ").class("section-title"));
    for s in &rep.by_section {
        let pct = if s.total == 0 { 0.0 } else { s.full as f64 * 100.0 / s.total as f64 };
        body = body.child(
            DecoratedBox::new().class("lp-item").child(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(s.name.clone()).class("lp-name"))
                    .child(DecoratedBox::new().class("grow"))
                    .child(Text::new(format!("разобрано {}", s.parsed)).class("lp-desc"))
                    .child(DecoratedBox::new().class("lp-badge lp-badge-on").child(Text::new(format!("{}/{} ({:.1}%)", s.full, s.total, pct)).class("lp-badge-text"))),
            ),
        );
    }

    // Недостающие инструменты
    if !rep.missing_tools.is_empty() {
        body = body.child(Text::new(format!("ТОП НЕДОСТАЮЩИХ ИНСТРУМЕНТОВ ({})", rep.missing_tools.len())).class("section-title"));
        for m in rep.missing_tools.iter().take(30) {
            let sole = if m.sole_blocker > 0 { format!("+{} до 100%", m.sole_blocker) } else { String::new() };
            let mut top_row = Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new(m.ty.clone()).class("lp-name"))
                .child(DecoratedBox::new().class("grow"))
                .child(DecoratedBox::new().class("lp-badge").child(Text::new(format!("{} шабл.", m.templates)).class("lp-badge-text")));
            if !sole.is_empty() {
                top_row = top_row.child(DecoratedBox::new().class("lp-badge lp-badge-on").child(Text::new(sole).class("lp-badge-text")));
            }
            body = body.child(
                DecoratedBox::new().class("lp-item").child(
                    Column::new()
                        .gap(3.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(top_row)
                        .child(Text::new(format!("пример: {}", m.sample)).max_lines(1).elide(Elide::Middle).class("lp-path")),
                ),
            );
        }
    }

    if !rep.parse_errors.is_empty() {
        body = body.child(Text::new(format!("ОШИБКИ РАЗБОРА ({})", rep.parse_errors.len())).class("section-title"));
        for (p, e) in rep.parse_errors.iter().take(15) {
            body = body.child(DecoratedBox::new().class("lp-item").child(Text::new(format!("{p}: {e}")).max_lines(2).class("lp-desc")));
        }
    }

    DecoratedBox::new()
        .class("fp-card")
        .child(
            Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(head)
                .child(summary)
                .child(DecoratedBox::new().class("grow").child(ScrollView::new().vertical().child(body))),
        )
        .style("width", 700.0f32)
        .style("height", 600.0f32)
}
