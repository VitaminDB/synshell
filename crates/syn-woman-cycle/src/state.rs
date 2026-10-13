//! Состояние «Цикла»: данные, прогноз, экраны, выбранный день.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use syngui::prelude::*;

use crate::date::{self, Day};
use crate::model::{self, Data, DayLog, Forecast, Settings};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Today,
    Calendar,
    History,
    Settings,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Today, Tab::Calendar, Tab::History, Tab::Settings];

    pub fn index(self) -> u64 {
        Tab::ALL.iter().position(|t| *t == self).unwrap_or(0) as u64
    }
}

#[derive(Clone, Copy)]
pub struct St {
    pub data: RwSignal<Data>,
    pub fc: RwSignal<Forecast>,
    /// Номер правки данных (перерисовка месяцев без перелистывания).
    pub rev: RwSignal<u64>,
    pub today: RwSignal<Day>,
    pub tab: RwSignal<Tab>,
    /// Показанный месяц календаря (`date::month_index`).
    pub month: RwSignal<i32>,
    /// День, открытый в карточке.
    pub sheet: RwSignal<Option<Day>>,
    pub sheet_open: RwSignal<bool>,
    pub confirm_reset: RwSignal<bool>,
    /// Отметка близости, открытая для правки (время, защита).
    pub love_edit: RwSignal<Option<u64>>,
    /// Шаг знакомства (0..3) и выбранная в нём дата.
    pub intro_step: RwSignal<u64>,
    pub intro_day: RwSignal<Option<Day>>,
    pub intro_month: RwSignal<i32>,
    pub window: RwSignal<syngui::window::WindowState>,
    pub theme: RwSignal<String>,
    pub dark: RwSignal<bool>,
    /// Масштаб интерфейса (живое значение, в настройки пишется после паузы).
    pub scale: RwSignal<f32>,
    pub toast: RwSignal<Option<(u64, String)>>,
}

static TOAST: AtomicU64 = AtomicU64::new(0);

impl St {
    pub fn new(data: Data) -> Self {
        let today = date::today();
        let fc = Forecast::new(&data, today);
        let scale = data.settings.scale;
        St {
            data: use_signal(data),
            fc: use_signal(fc),
            rev: use_signal(0),
            today: use_signal(today),
            tab: use_signal(Tab::Today),
            month: use_signal(date::month_index(today)),
            sheet: use_signal(None),
            sheet_open: use_signal(false),
            confirm_reset: use_signal(false),
            love_edit: use_signal(None),
            intro_step: use_signal(0),
            intro_day: use_signal(None),
            intro_month: use_signal(date::month_index(today)),
            window: use_signal(syngui::window::WindowState::default()),
            theme: use_signal(String::new()),
            dark: use_signal(false),
            scale: use_signal(scale),
            toast: use_signal(None),
        }
    }

    /// Смена даты в полночь: прогноз пересчитывается.
    pub fn start(self) {
        let st = self;
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(30));
            let d = date::today();
            run_on_main_thread(move || {
                if st.today.get_untracked() != d {
                    st.today.set(d);
                    st.recompute();
                }
            });
        });
    }

    fn recompute(&self) {
        let fc = Forecast::new(&self.data.get_untracked(), self.today.get_untracked());
        self.fc.set(fc);
        self.rev.set(self.rev.get_untracked() + 1);
    }

    /// Изменить данные, сохранить и пересчитать прогноз.
    pub fn edit(&self, f: impl FnOnce(&mut Data)) {
        let mut d = self.data.get_untracked();
        f(&mut d);
        model::save(&d);
        self.data.set(d);
        self.recompute();
    }

    pub fn settings(&self) -> Settings {
        self.data.get_untracked().settings
    }

    pub fn set_settings(&self, f: impl FnOnce(&mut Settings)) {
        self.edit(|d| f(&mut d.settings));
    }

    pub fn set_log(&self, day: Day, f: impl FnOnce(&mut DayLog)) {
        self.edit(|d| {
            let mut l = d.days.get(&day).copied().unwrap_or_default();
            f(&mut l);
            if l.is_empty() {
                d.days.remove(&day);
            } else {
                d.days.insert(day, l);
            }
        });
    }

    /// Месячные начались в этот день: отметка дня (обильность по умолчанию — средние).
    pub fn toggle_period(&self, day: Day) {
        let on = self.fc.get_untracked().log(day).flow > 0;
        self.edit(|d| {
            let l = d.days.entry(day).or_default();
            l.flow = if on { 0 } else { 3 };
            if l.is_empty() {
                d.days.remove(&day);
            }
            if !on && d.ended.is_some_and(|e| e <= day) {
                d.ended = None;
            }
        });
    }

    /// Месячные закончились: снять отметки после `day`, остаток прогноза скрыть.
    pub fn end_period(&self, day: Day) {
        self.edit(|d| {
            d.ended = Some(day);
            let tail: Vec<Day> = d.days.range(day + 1..day + 15).filter(|(_, l)| l.flow > 0).map(|(&k, _)| k).collect();
            for k in tail {
                if let Some(l) = d.days.get_mut(&k) {
                    l.flow = 0;
                    if l.is_empty() {
                        d.days.remove(&k);
                    }
                }
            }
        });
    }

    /// Отметить близость в день; сегодня — с текущим временем (его можно убрать).
    pub fn add_intimacy(&self, day: Day) -> u64 {
        let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        let time = (day == date::today()).then(date::now_minutes);
        self.edit(|d| d.intimacy.push(crate::model::Intimacy { id, day, time, protected: None }));
        id
    }

    pub fn update_intimacy(&self, id: u64, f: impl FnOnce(&mut crate::model::Intimacy)) {
        self.edit(|d| {
            if let Some(i) = d.intimacy.iter_mut().find(|i| i.id == id) {
                f(i);
            }
        });
    }

    pub fn remove_intimacy(&self, id: u64) {
        self.edit(|d| d.intimacy.retain(|i| i.id != id));
        if self.love_edit.get_untracked() == Some(id) {
            self.love_edit.set(None);
        }
    }

    pub fn open_day(&self, d: Day) {
        self.love_edit.set(None);
        self.sheet.set(Some(d));
        self.sheet_open.set(true);
    }

    pub fn go(&self, t: Tab) {
        self.sheet_open.set(false);
        self.tab.set(t);
    }

    /// «Назад»: закрыть карточку или диалог, вернуться на «Сегодня»; `false` — некуда.
    pub fn back(&self) -> bool {
        if self.confirm_reset.get_untracked() {
            self.confirm_reset.set(false);
            return true;
        }
        if self.sheet_open.get_untracked() {
            self.sheet_open.set(false);
            return true;
        }
        if !self.data.get_untracked().onboarded {
            let s = self.intro_step.get_untracked();
            if s > 0 {
                self.intro_step.set(s - 1);
                return true;
            }
            return false;
        }
        if self.tab.get_untracked() != Tab::Today {
            self.go(Tab::Today);
            return true;
        }
        false
    }

    pub fn shift_month(&self, by: i32) {
        self.month.set(self.month.get_untracked() + by);
    }

    pub fn toast(&self, msg: String) {
        let id = TOAST.fetch_add(1, Ordering::Relaxed) + 1;
        self.toast.set(Some((id, msg)));
        let t = self.toast;
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(2600));
            run_on_main_thread(move || {
                if t.get_untracked().is_some_and(|(i, _)| i == id) {
                    t.set(None);
                }
            });
        });
    }

    /// Данные знакомства как будто уже сохранены: выбранный день и длина месячных из настроек.
    fn intro_data(&self) -> Data {
        let mut d = self.data.get_untracked();
        if let Some(start) = self.intro_day.get_untracked() {
            let today = date::today();
            for i in 0..d.settings.period_len {
                if start + i > today {
                    break;
                }
                d.days.entry(start + i).or_default().flow = 3;
            }
        }
        d
    }

    /// Прогноз по выбору в знакомстве — для календаря-предпросмотра.
    pub fn intro_preview(&self) -> Forecast {
        Forecast::new(&self.intro_data(), date::today())
    }

    /// Завершить знакомство: отметить последние месячные (если дата выбрана).
    pub fn finish_intro(&self) {
        let mut d = self.intro_data();
        d.onboarded = true;
        model::save(&d);
        self.data.set(d);
        self.recompute();
        self.tab.set(Tab::Today);
    }

    pub fn reset(&self) {
        self.edit(|d| {
            let s = d.settings.clone();
            *d = Data { settings: s, ..Default::default() };
        });
        self.intro_step.set(0);
        self.intro_day.set(None);
        self.confirm_reset.set(false);
    }
}
