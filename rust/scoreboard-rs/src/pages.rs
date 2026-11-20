//! Port of `page-index` (scoretable) + `page-dashboard` (teamgrid).

use crate::models::*;
use crate::store::Store;
use crate::ui::Ui;
use leptos::prelude::*;

/// floatToString: short point sums, ≤1 decimal place.
fn f2s(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        let s = format!("{v:.1}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// `ScoretableComponent`: teams × services for one tick.
pub fn scoretable(store: Store, ui: Ui) -> impl IntoView {
    let shown_tick = RwSignal::new(-1i64);

    // follow newest tick until the user navigates manually
    Effect::new(move || {
        let newest = store.newest_scoreboard_tick.get();
        let current = store.state.get().current_tick;
        if shown_tick.get_untracked() < 0 {
            shown_tick.set(newest.min(current));
        }
    });

    let info = Memo::new(move |_| {
        let tick = shown_tick.get();
        if tick < 0 {
            None
        } else {
            store.ranking(tick)
        }
    });

    let nav = move |d: i64| {
        let max = store
            .newest_scoreboard_tick
            .get()
            .min(store.state.get().current_tick)
            .max(0);
        shown_tick.set((shown_tick.get() + d).clamp(0, max));
    };

    view! {
        <div>
            <p>
                <a href="#" on:click=move |e| { e.prevent_default(); nav(1) }>{"« newer"}</a>
                {move || {
                    if shown_tick.get() < store.newest_scoreboard_tick.get().min(store.state.get().current_tick) {
                        " (showing old tick)".into_any()
                    } else {
                        view! { <span></span> }.into_any()
                    }
                }}
                <a href="#" on:click=move |e| { e.prevent_default(); nav(-1) }>{"» older"}</a>
            </p>
            {move || match info.get() {
                Some(info) => score_table(&store, &ui, info.clone()).into_any(),
                None => view! { <p class="loading">"loading…"</p> }.into_any(),
            }}
        </div>
    }
}

fn score_table(store: &Store, ui: &Ui, info: RoundInformation) -> impl IntoView {
    let show_history = ui.show_history;
    let show_only_sums = ui.show_only_sums;
    let show_images = ui.show_images;
    let mut ranks: Vec<Rank> = info.scoreboard.clone();
    ranks.sort_by_key(|r| r.rank);

    let services: Vec<String> = info.services.iter().map(|s| s.name.clone()).collect();
    let svc_names = services.clone();

    view! {
        <table class="score">
            <thead>
                <tr>
                    <th></th>
                    <th class="teamcell">"Team"</th>
                    <th>"Score"</th>
                    {svc_names.into_iter().map(|s| view! { <th>{s}</th> }).collect::<Vec<_>>()}
                </tr>
            </thead>
            <tbody>
                {ranks.into_iter().map(|rank| {
                    let rank_services = rank.services.clone();
                    let team = store.team(rank.team_id);
                    let name = team.as_ref().map(|t| t.name.clone()).unwrap_or_else(|| format!("Team {}", rank.team_id));
                    let logo = team.as_ref().map(|t| t.logo.clone()).unwrap_or_default();
                    let banned = store.banned(rank.team_id);
                    let team_id = rank.team_id;
                    view! {
                        <tr>
                            <td class="rankcell">
                                <h4 class={match rank.rank { 1 => "rank1", 2 => "rank2", 3 => "rank3", _ => "" }}>{rank.rank}</h4>
                            </td>
                            <td class={if banned { "teamcell blocked" } else { "teamcell" }}>
                                <a href={format!("/team/{team_id}")}>
                                    {move || {
                                        let (show, logo) = (show_images.get(), logo.clone());
                                        if show && logo != "false" && !logo.is_empty() {
                                            view! { <img src={format!("logos/{logo}")} alt="" /> }.into_any()
                                        } else { view! { <span></span> }.into_any() }
                                    }}
                                    {name.clone()}
                                </a>
                            </td>
                            <td>
                                <span class="points">{f2s(rank.points)}</span>
                                <Delta diff=rank.o + rank.d + rank.s />
                            </td>
                            {rank_services.into_iter().enumerate().map(|(_si, sr)| {
                                let status = sr.c.clone();
                                view! {
                                    <td class={format!("sla-col {}", status_to_class(&status))}>
                                        <StatusCell sr=sr.clone() show_only_sums show_history />
                                        {move || {
                                            if show_history.get() {
                                                let dots = sr.dc.iter().take(3).cloned().collect::<Vec<_>>();
                                                dots.iter().enumerate().map(|(i, st)| {
                                                    let idx = 3 - i;
                                                    view! { <span class={format!("label {} history-{idx}", status_to_class(st))}>{status_to_text(st)}</span> }
                                                }).collect_view().into_any()
                                            } else { view! { <span></span> }.into_any() }
                                        }}
                                    </td>
                                }
                            }).collect::<Vec<_>>()}
                        </tr>
                    }
                }).collect::<Vec<_>>()}
            </tbody>
        </table>
    }
}

#[component]
fn Delta(diff: f64) -> impl IntoView {
    view! {
        <span class="delta">
            {move || {
                if diff != 0.0 {
                    let sign = if diff > 0.0 { "+" } else { "-" };
                    view! { <span class={if diff < 0.0 { "neg" } else { "" }}>{format!("{sign}{}", f2s(diff.abs()))}</span> }.into_any()
                } else { view! { <span></span> }.into_any() }
            }}
        </span>
    }
}

#[component]
fn StatusCell(sr: ServiceResult, show_only_sums: RwSignal<bool>, show_history: RwSignal<bool>) -> impl IntoView {
    let sr0 = sr.clone();
    view! {
        <div>
            {move || {
                let sr = sr0.clone();
                if show_only_sums.get() {
                    view! { <span class="points">{f2s(sr.o + sr.d + sr.s)}</span> }.into_any()
                } else {
                    view! {
                        <span class="points">{f2s(sr.o)}</span>
                        <span class="points">{f2s(sr.d)}</span>
                        <span class="points">{f2s(sr.s)}</span>
                    }.into_any()
                }
            }}
        </div>
        <div class="team-status-row">
            {move || {
                let sr = sr.clone();
                let st = sr.c.clone();
                if show_history.get() {
                    view! {
                        <span class={format!("label {}", status_to_class(&st))} title={status_to_tooltip(&st).to_string()}>
                            {status_to_text(&st)}
                        </span>
                    }.into_any()
                } else {
                    view! {
                        <span class={format!("label {}", status_to_class(&st))} title={status_to_tooltip(&st).to_string()}></span>
                    }.into_any()
                }
            }}
        </div>
    }
}

/// `PageDashboard`: card grid of teams.
pub fn dashboard(store: Store) -> impl IntoView {
    view! {
        <div class="teamgrid">
            {move || {
                let st = store.state.get();
                let info = store.ranking(st.scoreboard_tick);
                let teams = store.teams.get();
                match info {
                    None => view! { <p class="loading">"loading…"</p> }.into_any(),
                    Some(info) => {
                        let mut ranks = info.scoreboard.clone();
                        ranks.sort_by_key(|r| r.rank);
                        ranks.into_iter().map(|rank| {
                            let team = teams.get(&rank.team_id.to_string()).cloned().unwrap_or_default();
                            let name = if team.name.is_empty() { format!("Team {}", rank.team_id) } else { team.name.clone() };
                            let banned = st.banned_teams.contains(&rank.team_id);
                            let rank_cls = match rank.rank { 1 => "rank1", 2 => "rank2", 3 => "rank3", _ => "" };
                            let total = rank.points;
                            let delta = rank.o + rank.d + rank.s;
                            let team_id = rank.team_id;
                            let rank_services = rank.services.clone();
                            view! {
                                <a class="team" href={format!("/team/{team_id}")}>
                                    <div class={format!("rank {rank_cls}")}>{rank.rank}</div>
                                    <div>
                                        <h4 class={if banned { "team-name blocked" } else { "team-name" }}>{name}</h4>
                                        <div class="team-total-points">
                                            {f2s(total)} <span class="delta"><small>{format!("+{}", f2s(delta))}</small></span>
                                        </div>
                                        <div class="team-status-row">
                                            {rank_services.into_iter().map(|sr| {
                                                let st = sr.c.clone();
                                                view! { <span class={format!("label {}", status_to_class(&st))} title={status_to_tooltip(&st).to_string()}></span> }
                                            }).collect_view()}
                                        </div>
                                    </div>
                                    <div class="team-logo-media">
                                        {move || {
                                            let logo = team.logo.clone();
                                            if show_images() && logo != "false" && !logo.is_empty() {
                                                view! { <img src={format!("logos/{logo}")} alt="" /> }.into_any()
                                            } else { view! { <span></span> }.into_any() }
                                        }}
                                    </div>
                                </a>
                            }
                        }).collect_view().into_any()
                    }
                }
            }}
        </div>
    }
}

// show_images is a ReadSignal from the Ui context; resolve it lazily here so the
// dashboard view does not need to thread the Ui through every closure.
fn show_images() -> bool {
    use_context::<Ui>().map(|u| u.show_images.get()).unwrap_or(true)
}
