//! Port of `saarctf_commons/metric_utils.py` — Influx line-protocol formatting
//! with byte-exact Python semantics (`str(float)` and `json.dumps` rules).

use std::io::Write;

pub enum MetricValue {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

/// Python `json.dumps(s)` (ensure_ascii): lowercase \uXXXX, surrogate pairs for astral chars.
pub fn py_json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c if (c as u32) <= 0x7e => out.push(c),
            c => {
                let cp = c as u32;
                if cp > 0xFFFF {
                    let v = cp - 0x10000;
                    out.push_str(&format!(
                        "\\u{:04x}\\u{:04x}",
                        0xD800 + (v >> 10),
                        0xDC00 + (v & 0x3FF)
                    ));
                } else {
                    out.push_str(&format!("\\u{:04x}", cp));
                }
            }
        }
    }
    out.push('"');
    out
}

/// Python `str(float)`: integral below 1e16 keeps a ".0", at 1e16 / below 1e-4
/// switches to `1e+16` / `1e-05` (always a sign, at least two exponent digits).
pub fn py_float_str(v: f64) -> String {
    if v.is_nan() {
        return "nan".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if v != 0.0 && v.abs() < 1e-4 {
        return py_exp_form(v);
    }
    if v.abs() >= 1e16 {
        return py_exp_form(v);
    }
    if v.fract() == 0.0 {
        format!("{v:.1}")
    } else {
        format!("{v}")
    }
}

fn py_exp_form(v: f64) -> String {
    // Rust's {:e} is shortest-roundtrip like Python's repr; only the exponent
    // spelling differs ("1e16" vs "1e+16").
    let s = format!("{v:e}");
    let (mantissa, exp) = s
        .split_once('e')
        .expect("INFALLIBLE: {:e} always has an exponent");
    let exp: i32 = exp
        .parse()
        .expect("INFALLIBLE: {:e} exponent is an integer");
    if exp < 0 {
        format!("{mantissa}e-{:02}", -exp)
    } else {
        format!("{mantissa}e+{:02}", exp)
    }
}

/// `_value_to_influx`: int (and bool — a Python bool is an int) → `{v}i`,
/// str → json.dumps, else str(v).
pub fn value_to_influx(value: &MetricValue) -> String {
    match value {
        // Python bools format as their repr inside the int branch: "Truei"/"Falsei"
        MetricValue::Bool(true) => "Truei".into(),
        MetricValue::Bool(false) => "Falsei".into(),
        MetricValue::Int(v) => format!("{v}i"),
        MetricValue::Str(s) => py_json_str(s),
        MetricValue::Float(v) => py_float_str(*v),
    }
}

/// `_ts_to_influx`: `int(ts * 1e9)` (datetime is handled by the caller — the
/// Python side converts via `.timestamp()` before this sees it).
pub fn ts_to_influx(ts: f64) -> String {
    format!("{}", (ts * 1_000_000_000.0) as i64)
}

pub trait Writer: Write {}

/// `MetricRecorder.record_many`: `<metric><attrs> <fields> <ts>\n`, attrs first
/// (`,k=v` each), then fields (`,k=v` joined by `,`), both in insertion order.
pub fn format_line(
    metric: &str,
    values: &[(String, MetricValue)],
    attributes: &[(String, MetricValue)],
    ts: f64,
) -> String {
    let mut line = String::from(metric);
    for (k, v) in attributes {
        line.push(',');
        line.push_str(k);
        line.push('=');
        line.push_str(&value_to_influx(v));
    }
    line.push(' ');
    for (i, (k, v)) in values.iter().enumerate() {
        if i > 0 {
            line.push(',');
        }
        line.push_str(k);
        line.push('=');
        line.push_str(&value_to_influx(v));
    }
    line.push(' ');
    line.push_str(&ts_to_influx(ts));
    line.push('\n');
    line
}

pub type Timestamp = Option<f64>;

pub trait MetricRecorder {
    fn get_writer(&mut self) -> &mut dyn Write;

    fn record(&mut self, metric: &str, value_name: &str, value: MetricValue, ts: Timestamp) {
        self.record_many(
            metric,
            vec![(value_name.to_string(), value)],
            ts,
            Vec::new(),
        );
    }

    fn record_many(
        &mut self,
        metric: &str,
        values: Vec<(String, MetricValue)>,
        ts: Timestamp,
        attributes: Vec<(String, MetricValue)>,
    ) {
        if values.is_empty() {
            return;
        }
        let ts = ts.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0)
        });
        let line = format_line(metric, &values, &attributes, ts);
        let writer = self.get_writer();
        let _ = writer.write_all(line.as_bytes());
        let _ = writer.flush();
    }
}

/// `TelegrafTailMetricsRecorder` — appends to a file, opened lazily.
pub struct TelegrafTailMetricsRecorder {
    filename: String,
    file: Option<std::fs::File>,
}

impl TelegrafTailMetricsRecorder {
    pub fn new(filename: impl Into<String>) -> Self {
        TelegrafTailMetricsRecorder {
            filename: filename.into(),
            file: None,
        }
    }
}

impl MetricRecorder for TelegrafTailMetricsRecorder {
    fn get_writer(&mut self) -> &mut dyn Write {
        if self.file.is_none() {
            self.file = Some(
                std::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(&self.filename)
                    .expect("INFALLIBLE in practice: metrics log file must be writable"),
            );
        }
        self.file.as_mut().unwrap()
    }
}

/// `TelegrafBufferMetricsRecorder` — writes into any sink (stdout, a socket, …).
pub struct TelegrafBufferMetricsRecorder {
    file: Box<dyn Write + Send>,
}

impl TelegrafBufferMetricsRecorder {
    pub fn new(file: Box<dyn Write + Send>) -> Self {
        TelegrafBufferMetricsRecorder { file }
    }
}

impl MetricRecorder for TelegrafBufferMetricsRecorder {
    fn get_writer(&mut self) -> &mut dyn Write {
        &mut self.file
    }
}

/// `Metrics` global proxy (Python had a module-level `MetricsProxy` instance).
static METRICS: std::sync::OnceLock<std::sync::Mutex<Option<Box<dyn MetricRecorder + Send>>>> =
    std::sync::OnceLock::new();

/// `setup_default_metrics` — from the `METRICS_LOGFILE` env var (`-` = stdout).
pub fn setup_default_metrics() {
    let recorder: Box<dyn MetricRecorder + Send> = match std::env::var("METRICS_LOGFILE") {
        Ok(f) if f == "-" => Box::new(TelegrafBufferMetricsRecorder::new(Box::new(std::io::stdout()))),
        Ok(f) => Box::new(TelegrafTailMetricsRecorder::new(f)),
        Err(_) => return,
    };
    let cell = METRICS.get_or_init(|| std::sync::Mutex::new(None));
    *cell.lock().unwrap() = Some(recorder);
}

/// `Metrics.record_many(...)` — no-op until `setup_default_metrics` ran.
pub fn metrics_record_many(
    metric: &str,
    values: Vec<(String, MetricValue)>,
    ts: Timestamp,
    attributes: Vec<(String, MetricValue)>,
) {
    if let Some(cell) = METRICS.get() {
        if let Some(recorder) = cell.lock().unwrap().as_mut() {
            recorder.record_many(metric, values, ts, attributes);
        }
    }
}
