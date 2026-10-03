//! Usage statistics: turns, agent time and cost per provider, and a
//! 26-week heatmap of activity.

use crate::app_state::AppState;
use chrono::{Datelike, Duration, Local, NaiveDate};
use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::*;
use std::collections::{BTreeMap, HashMap};

const WEEKS: i64 = 26;

#[derive(Default)]
struct Totals {
    turns: usize,
    millis: u64,
    cost: f64,
}

pub struct StatsView {
    app: Entity<AppState>,
    stats: Vec<elyra_core::TurnStat>,
}

impl StatsView {
    pub fn new(app: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            app,
            stats: Vec::new(),
        };
        this.refresh(cx);
        this
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.stats = self.app.read(cx).store.turn_stats().unwrap_or_default();
        cx.notify();
    }
}

fn format_duration(millis: u64) -> String {
    let minutes = millis / 60_000;
    if minutes >= 60 {
        format!("{} h {} min", minutes / 60, minutes % 60)
    } else {
        format!("{minutes} min")
    }
}

impl Render for StatsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut total = Totals::default();
        let mut by_provider: BTreeMap<String, Totals> = BTreeMap::new();
        let mut by_day: HashMap<NaiveDate, usize> = HashMap::new();
        for stat in &self.stats {
            let (provider, at, millis, cost) =
                (&stat.provider, &stat.at, &stat.duration_ms, &stat.cost_usd);
            for totals in [&mut total, by_provider.entry(provider.clone()).or_default()] {
                totals.turns += 1;
                totals.millis += millis.unwrap_or(0);
                totals.cost += cost.unwrap_or(0.);
            }
            *by_day
                .entry(at.with_timezone(&Local).date_naive())
                .or_default() += 1;
        }
        let state = self.app.read(cx);
        let threads = state.threads.len() + state.archived_threads().len();

        let card = |label: &str, value: String| {
            v_flex()
                .flex_1()
                .p_3()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(label.to_string()),
                )
                .child(
                    div()
                        .text_xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(value),
                )
        };

        // Heatmap: columns are weeks (oldest left), rows Monday..Sunday.
        let today = Local::now().date_naive();
        let start = today
            - Duration::days(today.weekday().num_days_from_monday() as i64)
            - Duration::weeks(WEEKS - 1);
        let max = by_day.values().copied().max().unwrap_or(0).max(1);
        let (muted, primary) = (cx.theme().muted, cx.theme().primary);
        let by_day = &by_day;
        let weeks = (0..WEEKS).map(move |week| {
            v_flex().gap(px(3.)).children((0..7).map(move |day| {
                let date = start + Duration::days(week * 7 + day);
                let count = by_day.get(&date).copied().unwrap_or(0);
                let level = if date > today {
                    None
                } else if count == 0 {
                    Some(0.)
                } else {
                    Some(0.25 + 0.75 * count as f32 / max as f32)
                };
                div()
                    .id(SharedString::from(format!("day-{date}")))
                    .size(px(12.))
                    .rounded(px(2.))
                    .bg(match level {
                        None => transparent_black(),
                        Some(0.) => muted,
                        Some(level) => primary.opacity(level),
                    })
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(format!(
                            "{} · {count} turns",
                            date.format("%a %d.%m.%Y")
                        ))
                        .build(window, cx)
                    })
            }))
        });

        let providers = by_provider.into_iter().map(|(provider, totals)| {
            let label = elyra_core::ProviderKind::parse(&provider)
                .map(|k| k.label().to_string())
                .unwrap_or(provider);
            h_flex()
                .gap_2()
                .py_1()
                .border_b_1()
                .border_color(cx.theme().border)
                .text_sm()
                .child(div().flex_1().child(label))
                .child(div().w(px(90.)).child(format!("{} turns", totals.turns)))
                .child(div().w(px(110.)).child(format_duration(totals.millis)))
                .child(div().w(px(80.)).child(format!("${:.2}", totals.cost)))
        });

        div().id("stats").size_full().overflow_y_scroll().child(
            v_flex()
                .p_4()
                .gap_4()
                .max_w(px(900.))
                .child(
                    h_flex()
                        .gap_2()
                        .child(Icon::new(IconName::ChartColumn).small())
                        .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("Usage")),
                )
                .child(
                    h_flex()
                        .gap_3()
                        .child(card("Threads", threads.to_string()))
                        .child(card("Agent turns", total.turns.to_string()))
                        .child(card("Agent time", format_duration(total.millis)))
                        .child(card("Reported cost", format!("${:.2}", total.cost))),
                )
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).child("Activity, last 26 weeks"))
                .child(h_flex().gap(px(3.)).children(weeks))
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).child("By provider"))
                .child(v_flex().children(providers))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Cost is what providers report per turn (Claude Code and some ACP agents); subscription plans may show $0."),
                ),
        )
    }
}
