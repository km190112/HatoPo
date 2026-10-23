//! 通知の入力欄 (新しく送るときと、既存の通知を直すときで共用)。

use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use eframe::egui::{self, RichText};
use egui_extras::DatePickerButton;
use hatopo_core::{NewNotice, Notice};

/// 日付と時刻。`on` が false なら「指定しない」。
#[derive(Clone, Debug, PartialEq)]
pub struct When {
    pub on: bool,
    pub date: NaiveDate,
    pub hour: u32,
    pub minute: u32,
}

impl When {
    fn off(date: NaiveDate, hour: u32, minute: u32) -> When {
        When {
            on: false,
            date,
            hour,
            minute,
        }
    }

    fn from(dt: Option<NaiveDateTime>, fallback: NaiveDate, hour: u32, minute: u32) -> When {
        match dt {
            Some(d) => When {
                on: true,
                date: d.date(),
                hour: d.hour(),
                minute: d.minute(),
            },
            None => When::off(fallback, hour, minute),
        }
    }

    fn value(&self) -> Option<NaiveDateTime> {
        self.on.then(|| {
            self.date.and_time(
                NaiveTime::from_hms_opt(self.hour.min(23), self.minute.min(59), 0)
                    .unwrap_or(NaiveTime::MIN),
            )
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NoticeForm {
    pub title: String,
    pub body: String,
    pub due_on: bool,
    pub due: NaiveDate,
    pub urls: [String; 3],
    pub start: When,
    pub end: When,
}

impl NoticeForm {
    pub fn new(today: NaiveDate) -> NoticeForm {
        NoticeForm {
            title: String::new(),
            body: String::new(),
            due_on: false,
            due: today,
            urls: Default::default(),
            start: When::off(today, 9, 0),
            end: When::off(today, 23, 59),
        }
    }

    pub fn from_notice(n: &Notice, today: NaiveDate) -> NoticeForm {
        let mut urls: [String; 3] = Default::default();
        for (slot, u) in urls.iter_mut().zip(&n.urls) {
            *slot = u.clone();
        }
        NoticeForm {
            title: n.title.clone(),
            body: n.body.clone(),
            due_on: n.due.is_some(),
            due: n.due.unwrap_or(today),
            urls,
            start: When::from(Some(n.start_at), today, 9, 0),
            end: When::from(n.end_at, today, 23, 59),
        }
    }

    /// 入力の誤り (件名が空・終了が開始より前など) は保存のときに core が日本語で返す。
    pub fn to_new(&self) -> NewNotice {
        NewNotice {
            title: self.title.clone(),
            body: self.body.clone(),
            due: self.due_on.then_some(self.due),
            urls: self.urls.to_vec(),
            start_at: self.start.value(),
            end_at: self.end.value(),
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, salt: &str) {
        egui::Grid::new(format!("form-{salt}"))
            .num_columns(2)
            .spacing([12.0, 8.0])
            .striped(false)
            .show(ui, |ui| {
                ui.label(RichText::new("件名 *").strong());
                ui.add(
                    egui::TextEdit::singleline(&mut self.title)
                        .hint_text("例: 10月分の工数表を入力してください")
                        .desired_width(f32::INFINITY),
                );
                ui.end_row();

                ui.label("本文");
                ui.add(
                    egui::TextEdit::multiline(&mut self.body)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY),
                );
                ui.end_row();

                ui.label("依頼期限");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.due_on, "指定する");
                    if self.due_on {
                        ui.add(
                            DatePickerButton::new(&mut self.due)
                                .id_salt(&format!("due-{salt}"))
                                .format("%Y/%m/%d"),
                        );
                    }
                });
                ui.end_row();

                for (i, u) in self.urls.iter_mut().enumerate() {
                    ui.label(format!("リンク {}", i + 1));
                    ui.add(
                        egui::TextEdit::singleline(u)
                            .hint_text(r"\\server\共有\ファイル.xlsx や https://…")
                            .desired_width(f32::INFINITY),
                    );
                    ui.end_row();
                }

                ui.label("通知開始");
                when_ui(ui, &mut self.start, &format!("start-{salt}"), "今すぐ");
                ui.end_row();

                ui.label("通知終了");
                when_ui(
                    ui,
                    &mut self.end,
                    &format!("end-{salt}"),
                    "完了されるまで出す",
                );
                ui.end_row();
            });
    }
}

fn when_ui(ui: &mut egui::Ui, w: &mut When, salt: &str, off_label: &str) {
    ui.horizontal(|ui| {
        ui.checkbox(&mut w.on, "日時を指定");
        if w.on {
            ui.add(
                DatePickerButton::new(&mut w.date)
                    .id_salt(salt)
                    .format("%Y/%m/%d"),
            );
            ui.add(
                egui::DragValue::new(&mut w.hour)
                    .range(0..=23)
                    .suffix(" 時"),
            );
            ui.add(
                egui::DragValue::new(&mut w.minute)
                    .range(0..=59)
                    .speed(5)
                    .suffix(" 分"),
            );
        } else {
            ui.label(RichText::new(off_label).weak());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use hatopo_core::{Origin, Target};

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
    }

    #[test]
    fn empty_form_means_now_until_done() {
        let n = NoticeForm::new(day(1)).to_new();
        assert_eq!((n.start_at, n.end_at, n.due), (None, None, None));
    }

    #[test]
    fn notice_round_trips_through_the_form() {
        let notice = Notice {
            schema: 1,
            id: "x".into(),
            revision: 3,
            target: Target::broadcast(),
            title: "停電".into(),
            body: "本文".into(),
            due: Some(day(5)),
            urls: vec!["a".into(), "b".into()],
            created_at: day(1).and_hms_opt(8, 0, 0).unwrap(),
            created_by: Origin {
                user: "a".into(),
                host: "b".into(),
            },
            start_at: day(2).and_hms_opt(9, 30, 0).unwrap(),
            end_at: Some(day(27).and_hms_opt(23, 59, 0).unwrap()),
            updated_at: None,
            updated_by: None,
            group: None,
        };
        let n = NoticeForm::from_notice(&notice, day(1)).to_new();
        assert_eq!(n.title, "停電");
        assert_eq!(n.due, Some(day(5)));
        assert_eq!(n.urls, ["a", "b", ""]);
        assert_eq!(n.start_at, Some(notice.start_at));
        assert_eq!(n.end_at, notice.end_at);
    }
}
