//! Port of `checker_runner/runners/eno.py` — EnoCheckerRunner, native async
//! (no Python subprocess). Talks the enochecker_core JSON protocol.

use crate::dispatcher::TaskMessage;
use crate::runners::CheckerRunOutput;
use base64::Engine;
use hmac::{Hmac, Mac};
use rand::seq::SliceRandom;
use rand::Rng;
use saarctf_commons_rs::config::Config;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const TIME_BUFFER: f64 = 5.0;
const MAC_LENGTH: usize = 16;

/// the three task-delay lists (early, late, unconstrained), in seconds
type TimeSlots = (Vec<f64>, Vec<f64>, Vec<f64>);

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckerInfoMessage {
    #[serde(default)]
    pub service_name: String,
    pub flag_variants: i64,
    pub noise_variants: i64,
    pub havoc_variants: i64,
    #[serde(default)]
    pub exploit_variants: i64,
    #[serde(default)]
    pub test_variants: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckerTaskMessage {
    task_id: i64,
    method: &'static str,
    address: String,
    team_id: i64,
    team_name: String,
    current_round_id: i64,
    related_round_id: i64,
    flag: Option<String>,
    variant_id: i64,
    timeout: i64,
    round_length: i64,
    task_chain_id: String,
    flag_regex: Option<String>,
    flag_hash: Option<String>,
    attack_info: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckerResultMessage {
    result: String,
    message: Option<String>,
    attack_info: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    flag: Option<String>,
}

/// `EnoCheckerRunner._service_info_cache` (ClassVar).
fn info_cache() -> &'static Mutex<HashMap<String, CheckerInfoMessage>> {
    static CACHE: OnceLock<Mutex<HashMap<String, CheckerInfoMessage>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// `EnoCheckerRunner.get_flag` — same `<HHHH` pack + HMAC-SHA256[:16] + b64
/// (urlsafe alphabet) as `gamelib.get_flag`, with the configured key/prefix.
fn eno_flag(cfg: &Config, service_id: i64, team_id: i64, tick: i64, payload: i64) -> String {
    let pack = |v: i64| -> [u8; 2] { (v as u16).to_le_bytes() };
    let mut data = Vec::with_capacity(8);
    data.extend_from_slice(&pack(tick & 0xffff));
    data.extend_from_slice(&pack(team_id));
    data.extend_from_slice(&pack(service_id & 0xffff));
    data.extend_from_slice(&pack(payload));
    let mut mac = Hmac::<Sha256>::new_from_slice(&cfg.secret_flag_key)
        .expect("INFALLIBLE: HMAC accepts any key length");
    mac.update(&data);
    let digest = mac.finalize().into_bytes();
    data.extend_from_slice(&digest[..MAC_LENGTH]);
    let encoded = base64::engine::general_purpose::STANDARD
        .encode(data)
        .replace('+', "-")
        .replace('/', "_");
    format!("{}{{{}}}", cfg.flag_prefix, encoded)
}

async fn get_tick_duration(
    redis: &mut redis::aio::ConnectionManager,
    tick: i64,
) -> Result<i64, ()> {
    let v: Option<String> = redis::cmd("GET")
        .arg(format!("round:{tick}:time"))
        .query_async(redis)
        .await
        .map_err(|_| ())?;
    v.and_then(|s| s.parse().ok()).ok_or(())
}

async fn get_fresh_task_id(redis: &mut redis::aio::ConnectionManager) -> i64 {
    redis::cmd("INCR")
        .arg("runner:eno:task_id")
        .query_async::<i64>(redis)
        .await
        .unwrap_or(0)
}

async fn set_flag_id(
    redis: &mut redis::aio::ConnectionManager,
    service_id: i64,
    team_id: i64,
    tick: i64,
    index: i64,
    value: &str,
) {
    let _: Result<(), _> = redis::cmd("SET")
        .arg(format!(
            "custom_flag_ids:{service_id}:{tick}:{team_id}:{index}"
        ))
        .arg(value)
        .query_async(redis)
        .await;
}

async fn get_flag_id(
    redis: &mut redis::aio::ConnectionManager,
    service_id: i64,
    team_id: i64,
    tick: i64,
    index: i64,
) -> Option<String> {
    redis::cmd("GET")
        .arg(format!(
            "custom_flag_ids:{service_id}:{tick}:{team_id}:{index}"
        ))
        .query_async(redis)
        .await
        .unwrap_or(None)
}

struct EnoRunner<'a> {
    service_id: i64,
    cfg: &'a Config,
    url: String,
    custom_results: HashMap<String, String>,
    output: Vec<String>,
    messages: HashMap<String, String>,
    recovery_messages: HashMap<String, String>,
    status: String,
}

impl<'a> EnoRunner<'a> {
    fn new(service_id: i64, cfg: &'a Config, url: String) -> Self {
        EnoRunner {
            service_id,
            cfg,
            url,
            custom_results: HashMap::new(),
            output: Vec::new(),
            messages: HashMap::new(),
            recovery_messages: HashMap::new(),
            status: "SUCCESS".into(),
        }
    }

    fn reset(&mut self) {
        self.custom_results.clear();
        self.output.clear();
        self.messages.clear();
        self.recovery_messages.clear();
        self.status = "SUCCESS".into();
    }

    async fn get_service_info(&self, http: &reqwest::Client) -> Result<CheckerInfoMessage, String> {
        if let Some(cached) = info_cache().lock().unwrap().get(&self.url) {
            return Ok(cached.clone());
        }
        let resp = http
            .get(format!("{}/service", self.url))
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|e| format!("Exception: {e}"))?;
        if resp.status() != reqwest::StatusCode::OK {
            return Err(format!(
                "Exception: Info request {}/service returned {}",
                self.url,
                resp.status()
            ));
        }
        let cim: CheckerInfoMessage = resp.json().await.map_err(|e| format!("Exception: {e}"))?;
        info_cache()
            .lock()
            .unwrap()
            .insert(self.url.clone(), cim.clone());
        Ok(cim)
    }

    /// `_message` — builds the task message for one checker call.
    async fn message(
        &self,
        redis: &mut redis::aio::ConnectionManager,
        team_id: i64,
        tick: i64,
        method: &'static str,
        variant_id: i64,
        related_tick: Option<i64>,
    ) -> CheckerTaskMessage {
        let tick_length = get_tick_duration(redis, tick).await.unwrap_or(60);
        let mut msg = CheckerTaskMessage {
            task_id: get_fresh_task_id(redis).await,
            method,
            address: self.cfg.network.team_id_to_vulnbox_ip(team_id),
            team_id,
            team_name: format!("#{team_id}"),
            current_round_id: tick,
            related_round_id: related_tick.unwrap_or(tick),
            flag: None,
            variant_id,
            timeout: (self.cfg.runner.eno.timeout.as_f64().unwrap_or(15.0) * 1000.0) as i64,
            round_length: tick_length * 1000,
            task_chain_id: String::new(),
            flag_regex: None,
            flag_hash: None,
            attack_info: None,
        };
        let rel = msg.related_round_id;
        match method {
            "putflag" | "getflag" => {
                msg.task_chain_id =
                    format!("flag_s{}_r{rel}_t{team_id}_i{variant_id}", self.service_id);
                msg.flag = Some(eno_flag(
                    self.cfg,
                    self.service_id,
                    team_id,
                    rel,
                    variant_id,
                ));
            }
            "putnoise" | "getnoise" => {
                msg.task_chain_id =
                    format!("noise_s{}_r{rel}_t{team_id}_i{variant_id}", self.service_id);
            }
            "havoc" => {
                msg.task_chain_id =
                    format!("havoc_s{}_r{rel}_t{team_id}_i{variant_id}", self.service_id);
            }
            "exploit" => {
                let flag = eno_flag(self.cfg, self.service_id, team_id, tick, variant_id);
                let mut hasher = Sha256::new();
                hasher.update(flag.as_bytes());
                msg.flag_hash = Some(hex::encode(hasher.finalize()));
                msg.flag_regex = Some(gamelib_rs::port::get_flag_regex_pattern());
                msg.attack_info =
                    get_flag_id(redis, self.service_id, team_id, tick, variant_id).await;
            }
            _ => {}
        }
        msg
    }

    /// `_task_chain_msg_write` — first message wins, OFFLINE overrides.
    fn task_chain_msg_write(
        map: &mut HashMap<String, String>,
        key: &str,
        msg: &CheckerResultMessage,
    ) {
        let text = msg.message.clone();
        if msg.result == "OFFLINE" {
            map.insert(key.to_string(), text.unwrap_or_default());
        } else {
            map.entry(key.to_string())
                .or_insert_with(|| text.unwrap_or_default());
        }
    }

    /// `_handle_result` — status transitions, flag-id storage, output line.
    async fn handle_result(
        &mut self,
        redis: &mut redis::aio::ConnectionManager,
        msg: &CheckerTaskMessage,
        result: &CheckerResultMessage,
    ) {
        if result.message.is_some() {
            if msg.current_round_id == msg.related_round_id {
                Self::task_chain_msg_write(&mut self.messages, &msg.task_chain_id, result);
            } else {
                Self::task_chain_msg_write(&mut self.recovery_messages, &msg.task_chain_id, result);
            }
        }

        if result.result == "INTERNAL_ERROR" {
            self.status = if result.message.as_deref() != Some("TIMEOUT") {
                "CRASHED".into()
            } else {
                "TIMEOUT".into()
            };
        } else if (self.status == "SUCCESS" || self.status == "RECOVERING")
            && result.result != "OK"
            && msg.current_round_id != msg.related_round_id
        {
            self.status = "RECOVERING".into();
        } else if (self.status == "SUCCESS" || self.status == "RECOVERING")
            && result.result == "MUMBLE"
        {
            self.status = "MUMBLE".into();
        } else if (self.status == "SUCCESS"
            || self.status == "RECOVERING"
            || self.status == "MUMBLE")
            && result.result == "OFFLINE"
        {
            self.status = "OFFLINE".into();
        }

        // store flag IDs / attack info
        if msg.method == "putflag" {
            if let Some(attack_info) = &result.attack_info {
                set_flag_id(
                    redis,
                    self.service_id,
                    msg.team_id,
                    msg.current_round_id,
                    msg.variant_id,
                    attack_info,
                )
                .await;
            }
        }

        self.output.push(format!(
            "{:8} #{} for tick {} => {}",
            msg.method.to_uppercase(),
            msg.variant_id,
            msg.related_round_id,
            result.result
        ));
    }

    /// `gen_timeslots_flat` — Err(()) on a failed clearance assert.
    fn gen_timeslots_flat(
        tick_length: f64,
        flag_variants: i64,
        noise_variants: i64,
        havoc_variants: i64,
        check_past_ticks: i64,
        task_timeout_s: f64,
    ) -> Result<TimeSlots, ()> {
        let mut rng = rand::thread_rng();
        let total_tasks_count =
            (2 + check_past_ticks) * flag_variants + 2 * noise_variants + havoc_variants;
        let constrained_task_count = flag_variants + noise_variants;
        if total_tasks_count < 2 * constrained_task_count {
            return Err(());
        }

        let last_possible_start_offset = tick_length - TIME_BUFFER - task_timeout_s;
        let task_start_interval = last_possible_start_offset / total_tasks_count as f64;
        let task_interval_start_offsets: Vec<f64> = (0..total_tasks_count)
            .map(|i| task_start_interval * i as f64)
            .collect();

        let latest_early_task_start_offset =
            last_possible_start_offset - 2.0 * task_start_interval - task_timeout_s;
        let earliest_late_task_start_offset = task_timeout_s + task_start_interval;

        let early: Vec<f64> =
            task_interval_start_offsets[..constrained_task_count as usize].to_vec();
        let late: Vec<f64> = task_interval_start_offsets
            [(total_tasks_count - constrained_task_count) as usize..]
            .to_vec();
        let unconstrained: Vec<f64> = task_interval_start_offsets[constrained_task_count as usize
            ..(total_tasks_count - constrained_task_count) as usize]
            .to_vec();

        // the Python asserts — any failure falls back to the simple generator
        let early_last = *early.last().ok_or(())?;
        let early_first = *early.first().ok_or(())?;
        let late_first = *late.first().ok_or(())?;
        if latest_early_task_start_offset < early_last
            || earliest_late_task_start_offset > late_first
            || early_first + task_timeout_s + task_start_interval > late_first
        {
            return Err(());
        }

        #[derive(Clone, Copy)]
        enum SlotType {
            Early,
            Unconstrained,
            Late,
        }
        #[derive(Clone, Copy)]
        struct Entry {
            entry_type: SlotType,
            offset: f64,
            idx: usize,
        }
        let mut early_tagged: Vec<Entry> = early
            .iter()
            .enumerate()
            .map(|(idx, val)| Entry {
                entry_type: SlotType::Early,
                offset: *val,
                idx,
            })
            .collect();
        let mut unconstrained_tagged: Vec<Entry> = unconstrained
            .iter()
            .enumerate()
            .map(|(idx, val)| Entry {
                entry_type: SlotType::Unconstrained,
                offset: *val,
                idx,
            })
            .collect();
        let mut late_tagged: Vec<Entry> = late
            .iter()
            .enumerate()
            .map(|(idx, val)| Entry {
                entry_type: SlotType::Late,
                offset: *val,
                idx,
            })
            .collect();

        let mut tagged_task_slots: Vec<Entry> = Vec::new();
        tagged_task_slots.extend(early_tagged.iter().copied());
        tagged_task_slots.extend(unconstrained_tagged.iter().copied());
        tagged_task_slots.extend(late_tagged.iter().copied());

        let clearance = task_timeout_s + task_start_interval;
        let valid_clearance = |early_task: &Entry, late_task: &Entry| {
            late_task.offset - early_task.offset >= clearance
        };
        // `_try_perform_swap` — offsets live in the tagged vectors by (type, idx)
        let try_perform_swap = |early: &mut Vec<Entry>,
                                unc: &mut Vec<Entry>,
                                late: &mut Vec<Entry>,
                                current: &Entry,
                                swap_target: &Entry| {
            let get = |e: &Entry| match e.entry_type {
                SlotType::Early => early[e.idx].offset,
                SlotType::Unconstrained => unc[e.idx].offset,
                SlotType::Late => late[e.idx].offset,
            };
            let swapped = match swap_target.entry_type {
                SlotType::Unconstrained => true,
                SlotType::Early => {
                    let swap_target_late = late[swap_target.idx];
                    valid_clearance(current, &swap_target_late)
                }
                SlotType::Late => {
                    let swap_target_early = early[swap_target.idx];
                    valid_clearance(&swap_target_early, current)
                }
            };
            if swapped {
                let (a, b) = (get(current), get(swap_target));
                let mut set = |e: &Entry, v: f64| match e.entry_type {
                    SlotType::Early => early[e.idx].offset = v,
                    SlotType::Unconstrained => unc[e.idx].offset = v,
                    SlotType::Late => late[e.idx].offset = v,
                };
                set(current, b);
                set(swap_target, a);
            }
        };

        for _ in 0..20 {
            for constrained_task_idx in 0..constrained_task_count as usize {
                let early_task = early_tagged[constrained_task_idx];
                let current_late_task = late_tagged[early_task.idx];
                let swap_late_task = *tagged_task_slots.choose(&mut rng).ok_or(())?;

                if valid_clearance(&early_task, &swap_late_task) {
                    try_perform_swap(
                        &mut early_tagged,
                        &mut unconstrained_tagged,
                        &mut late_tagged,
                        &current_late_task,
                        &swap_late_task,
                    );
                }

                let late_idx = late_tagged.len() - constrained_task_idx - 1;
                let late_task = late_tagged[late_idx];
                let current_early_task = early_tagged[late_task.idx];
                let swap_early_task = *tagged_task_slots.choose(&mut rng).ok_or(())?;
                if valid_clearance(&swap_early_task, &late_task) {
                    try_perform_swap(
                        &mut early_tagged,
                        &mut unconstrained_tagged,
                        &mut late_tagged,
                        &current_early_task,
                        &swap_early_task,
                    );
                }
            }
        }

        let mut early_late: Vec<(f64, f64)> = early_tagged
            .iter()
            .zip(late_tagged.iter())
            .map(|(e, l)| (e.offset, l.offset))
            .collect();
        let mut unconstrained_offsets: Vec<f64> =
            unconstrained_tagged.iter().map(|u| u.offset).collect();

        early_late.shuffle(&mut rng);
        unconstrained_offsets.shuffle(&mut rng);

        let early_slots: Vec<f64> = early_late.iter().map(|x| x.0).collect();
        let late_slots: Vec<f64> = early_late.iter().map(|x| x.1).collect();

        // jitter within the interval
        let mut jitter = |slots: &[f64]| -> Vec<f64> {
            slots
                .iter()
                .map(|i| rng.gen::<f64>() * task_start_interval + i)
                .collect()
        };
        Ok((
            jitter(&early_slots),
            jitter(&late_slots),
            jitter(&unconstrained_offsets),
        ))
    }

    /// `gen_timeslots_fallback`
    fn gen_timeslots_fallback(
        flag_variants: i64,
        noise_variants: i64,
        havoc_variants: i64,
        check_past_ticks: i64,
    ) -> TimeSlots {
        let mut rng = rand::thread_rng();
        let unconstrained_task_count = check_past_ticks * flag_variants + havoc_variants;
        let constrained_task_count = flag_variants + noise_variants;
        let early = (0..constrained_task_count)
            .map(|_| rng.gen::<f64>() * 15.0)
            .collect();
        let late = (0..constrained_task_count)
            .map(|_| 30.0 + rng.gen::<f64>() * 10.0)
            .collect();
        let unconstrained = (0..unconstrained_task_count)
            .map(|_| rng.gen::<f64>() * 40.0)
            .collect();
        (early, late, unconstrained)
    }

    /// `gen_timeslots`
    async fn gen_timeslots(
        redis: &mut redis::aio::ConnectionManager,
        cfg: &Config,
        tick: i64,
        info: &CheckerInfoMessage,
        task_timeout_s: f64,
    ) -> TimeSlots {
        let tick_length = get_tick_duration(redis, tick).await.unwrap_or(60) as f64;
        let check_past = cfg.runner.eno.check_past_ticks.as_i64().unwrap_or(5);
        Self::gen_timeslots_flat(
            tick_length,
            info.flag_variants,
            info.noise_variants,
            info.havoc_variants,
            check_past,
            task_timeout_s,
        )
        .unwrap_or_else(|_| {
            Self::gen_timeslots_fallback(
                info.flag_variants,
                info.noise_variants,
                info.havoc_variants,
                check_past,
            )
        })
    }
}

/// `_query` — POST one task message; timeout → TIMEOUT, other errors →
/// INTERNAL_ERROR with a traceback line appended to `output`.
async fn query_once(
    http: &reqwest::Client,
    url: &str,
    output: &mut Vec<String>,
    msg: &CheckerTaskMessage,
) -> CheckerResultMessage {
    let body = serde_json::to_string(msg).unwrap_or_default();
    let timeout = Duration::from_millis(msg.timeout as u64 + 1000);
    let send = http
        .post(url)
        .header("Content-Type", "application/json")
        .timeout(timeout)
        .body(body)
        .send()
        .await;
    let response = match send {
        Ok(r) => r,
        Err(e) if e.is_timeout() => {
            return CheckerResultMessage {
                result: "INTERNAL_ERROR".into(),
                message: Some("TIMEOUT".into()),
                attack_info: None,
                flag: None,
            };
        }
        Err(e) => {
            output.push(format!("Traceback: request to {url} failed: {e}"));
            return CheckerResultMessage {
                result: "INTERNAL_ERROR".into(),
                message: None,
                attack_info: None,
                flag: None,
            };
        }
    };
    if response.status() != reqwest::StatusCode::OK {
        output.push(format!(
            "Traceback: Request to {url} returned with code {}",
            response.status()
        ));
        return CheckerResultMessage {
            result: "INTERNAL_ERROR".into(),
            message: None,
            attack_info: None,
            flag: None,
        };
    }
    match response.json::<CheckerResultMessage>().await {
        Ok(result) => result,
        Err(e) => {
            output.push(format!("Traceback: {e}"));
            CheckerResultMessage {
                result: "INTERNAL_ERROR".into(),
                message: None,
                attack_info: None,
                flag: None,
            }
        }
    }
}

/// `execute_checker_async`: build all task messages with their slot delays,
/// run the HTTP calls concurrently, then apply the results.
async fn execute(
    runner: &mut EnoRunner<'_>,
    redis: &mut redis::aio::ConnectionManager,
    team_id: i64,
    tick: i64,
) -> CheckerRunOutput {
    runner.reset();
    let http = reqwest::Client::new();
    let info = match runner.get_service_info(&http).await {
        Ok(info) => info,
        Err(e) => {
            // Python: the exception fails the celery task → CRASHED row
            return CheckerRunOutput {
                status: "CRASHED".into(),
                message: None,
                output: Some(e),
                data: None,
            };
        }
    };

    let (early_slots, late_slots, unconstrained_slots) =
        EnoRunner::gen_timeslots(redis, runner.cfg, tick, &info, 15.0).await;
    let mut early_iter = early_slots.into_iter();
    let mut late_iter = late_slots.into_iter();
    let mut unconstrained_iter = unconstrained_slots.into_iter();

    let mut jobs: Vec<(CheckerTaskMessage, f64)> = Vec::new();
    let check_past = runner.cfg.runner.eno.check_past_ticks.as_i64().unwrap_or(5);

    for variant_id in 0..info.flag_variants {
        let msg = runner
            .message(redis, team_id, tick, "putflag", variant_id, None)
            .await;
        jobs.push((msg, early_iter.next().unwrap_or(0.0)));
        let msg = runner
            .message(redis, team_id, tick, "getflag", variant_id, None)
            .await;
        jobs.push((msg, late_iter.next().unwrap_or(0.0)));
        for i in 1..=check_past {
            if tick - i <= 0 {
                break;
            }
            let msg = runner
                .message(redis, team_id, tick, "getflag", variant_id, Some(tick - i))
                .await;
            jobs.push((msg, unconstrained_iter.next().unwrap_or(0.0)));
        }
    }
    for variant_id in 0..info.noise_variants {
        let msg = runner
            .message(redis, team_id, tick, "putnoise", variant_id, None)
            .await;
        jobs.push((msg, early_iter.next().unwrap_or(0.0)));
        let msg = runner
            .message(redis, team_id, tick, "getnoise", variant_id, None)
            .await;
        jobs.push((msg, late_iter.next().unwrap_or(0.0)));
    }
    for variant_id in 0..info.havoc_variants {
        let msg = runner
            .message(redis, team_id, tick, "havoc", variant_id, None)
            .await;
        jobs.push((msg, unconstrained_iter.next().unwrap_or(0.0)));
    }

    // all queries in parallel, results applied as they come in
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    for (msg, delay) in jobs {
        let http = http.clone();
        let url = runner.url.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs_f64(delay.max(0.0))).await;
            let mut scratch = Vec::new();
            let result = query_once(&http, &url, &mut scratch, &msg).await;
            let _ = tx.send((msg, result, scratch));
        });
    }
    drop(tx);
    while let Some((msg, result, scratch)) = rx.recv().await {
        runner.output.extend(scratch);
        if msg.method == "getflag" {
            runner.custom_results.insert(
                format!("{}_{}", msg.related_round_id, msg.variant_id),
                result.result.clone(),
            );
        }
        runner.handle_result(redis, &msg, &result).await;
    }

    let msgs = if runner.messages.is_empty() {
        runner
            .recovery_messages
            .values()
            .cloned()
            .collect::<Vec<_>>()
    } else {
        runner.messages.values().cloned().collect::<Vec<_>>()
    };
    let unique: BTreeSet<String> = msgs.into_iter().collect();
    runner.output.sort();
    CheckerRunOutput {
        status: runner.status.clone(),
        message: Some(unique.into_iter().collect::<Vec<_>>().join("\n")),
        output: Some(runner.output.join("\n")),
        data: Some(serde_json::to_value(&runner.custom_results).unwrap_or_default()),
    }
}

/// Entry point for `runners::run_checker`: the `eno:*` dispatch branch.
pub async fn run_eno_checker(task: &TaskMessage, cfg: &Config) -> CheckerRunOutput {
    let url = match task
        .runner_config
        .as_ref()
        .and_then(|c| c.get("url"))
        .and_then(|u| u.as_str())
    {
        Some(url) => url.to_string(),
        None => {
            return CheckerRunOutput {
                status: "CRASHED".into(),
                message: None,
                output: Some("Exception: No checker url set (runner_config.url missing)".into()),
                data: None,
            };
        }
    };
    let mut redis = match saarctf_commons_rs::redis::get_redis_connection(&cfg.redis).await {
        Ok(conn) => conn,
        Err(e) => {
            return CheckerRunOutput {
                status: "CRASHED".into(),
                message: None,
                output: Some(format!("Exception: redis connect: {e}")),
                data: None,
            };
        }
    };
    let mut runner = EnoRunner::new(task.service_id, cfg, url);
    execute(&mut runner, &mut redis, task.team_id, task.tick).await
}
