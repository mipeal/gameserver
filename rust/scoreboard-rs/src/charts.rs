//! Thin wasm-bindgen shim over the vendored chart.js 4.x UMD build.

use wasm_bindgen::prelude::*;

/// Current value of a CSS custom property on :root (charts cannot read CSS
/// vars themselves); falls back when unavailable.
fn css_var(name: &str, fallback: &str) -> String {
    let Some(win) = web_sys::window() else { return fallback.into() };
    let Some(doc) = win.document() else { return fallback.into() };
    let Some(root) = doc.document_element() else { return fallback.into() };
    let Ok(Some(style)) = win.get_computed_style(&root) else { return fallback.into() };
    let v = style.get_property_value(name).unwrap_or_default();
    if v.trim().is_empty() { fallback.into() } else { v }
}

#[wasm_bindgen(module = "/js/chart.umd.js")]
extern "C" {
    type Chart;

    #[wasm_bindgen(constructor)]
    fn new(canvas: &web_sys::HtmlCanvasElement, config: &JsValue) -> Chart;

    #[wasm_bindgen(method)]
    fn destroy(this: &Chart);
}

/// A live chart; destroying on drop keeps page switches leak-free.
pub struct ChartHandle {
    inner: Option<Chart>,
}

impl ChartHandle {
    pub fn line(
        canvas: &web_sys::HtmlCanvasElement,
        labels: &[String],
        datasets: &JsValue,
        scales: &JsValue,
    ) -> ChartHandle {
        let config = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&config, &"type".into(), &"line".into());
        let data = js_sys::Object::new();
        let labels_arr = labels
            .iter()
            .map(|l| JsValue::from_str(l))
            .collect::<js_sys::Array>();
        let _ = js_sys::Reflect::set(&data, &"labels".into(), &labels_arr);
        let _ = js_sys::Reflect::set(&data, &"datasets".into(), datasets);
        let _ = js_sys::Reflect::set(&config, &"data".into(), &data);
        let options = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&options, &"maintainAspectRatio".into(), &false.into());
        // no draw-in animation: charts are rebuilt on every tick and the
        // animation would restart from zero each time
        let _ = js_sys::Reflect::set(&options, &"animation".into(), &false.into());
        let _ = js_sys::Reflect::set(&options, &"responsive".into(), &true.into());
        // Ops Console theme: default text color for ticks + legend labels
        let _ = js_sys::Reflect::set(&options, &"color".into(), &css_var("--dim", "#8b949e").into());
        // chartOptions.interaction, as in the original
        let interaction = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&interaction, &"mode".into(), &"nearest".into());
        let _ = js_sys::Reflect::set(&interaction, &"axis".into(), &"x".into());
        let _ = js_sys::Reflect::set(&interaction, &"intersect".into(), &false.into());
        let _ = js_sys::Reflect::set(&options, &"interaction".into(), &interaction.into());
        // restrained legend: small swatches, dim labels
        let legend_labels = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&legend_labels, &"boxWidth".into(), &8.into());
        let _ = js_sys::Reflect::set(&legend_labels, &"boxHeight".into(), &8.into());
        let _ = js_sys::Reflect::set(&legend_labels, &"padding".into(), &10.into());
        let legend_font = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&legend_font, &"size".into(), &10.into());
        let _ = js_sys::Reflect::set(&legend_labels, &"font".into(), &legend_font);
        let legend = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&legend, &"labels".into(), &legend_labels);
        let _ = js_sys::Reflect::set(&options, &"legend".into(), &legend);
        let scales_obj = js_sys::Object::from(scales.clone());
        let _ = js_sys::Reflect::set(&options, &"scales".into(), &scales_obj);
        let _ = js_sys::Reflect::set(&config, &"options".into(), &options);
        ChartHandle {
            inner: Some(Chart::new(canvas, &config)),
        }
    }
}

impl Drop for ChartHandle {
    fn drop(&mut self) {
        if let Some(c) = self.inner.take() {
            c.destroy();
        }
    }
}

/// Build one line dataset.
pub fn dataset(
    label: &str,
    data: &[f64],
    color: &str,
    fill: bool,
    stack: Option<&str>,
    border_dash: Option<&[u8]>,
) -> JsValue {
    let obj = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&obj, &"label".into(), &label.into());
    let _ = js_sys::Reflect::set(
        &obj,
        &"data".into(),
        &data.iter().map(|f| JsValue::from(*f)).collect::<js_sys::Array>(),
    );
    let _ = js_sys::Reflect::set(&obj, &"borderColor".into(), &color.into());
    if fill {
        // addScheme (light scheme): backgroundColor = color lightened by 0.2
        let bg: JsValue = lighten(color, 0.2).into();
        let _ = js_sys::Reflect::set(&obj, &"backgroundColor".into(), &bg);
        let _ = js_sys::Reflect::set(&obj, &"pointBorderColor".into(), &bg);
        let _ = js_sys::Reflect::set(&obj, &"pointHoverBackgroundColor".into(), &bg);
        let fill_js = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&fill_js, &"target".into(), &"origin".into());
        let _ = js_sys::Reflect::set(&obj, &"fill".into(), &fill_js.into());
    } else {
        let _ = js_sys::Reflect::set(&obj, &"fill".into(), &false.into());
    }
    let _ = js_sys::Reflect::set(&obj, &"pointRadius".into(), &0.into());
    if let Some(stack) = stack {
        let _ = js_sys::Reflect::set(&obj, &"stack".into(), &stack.into());
    }
    if let Some(dash) = border_dash {
        let _ = js_sys::Reflect::set(
            &obj,
            &"borderDash".into(),
            &dash.iter().map(|d| JsValue::from(*d)).collect::<js_sys::Array>(),
        );
    }
    obj.into()
}

/// '#rrggbb' lightened by `amount` in HSL space (kurkle color lighten).
fn lighten(color: &str, amount: f64) -> String {
    fn hue_to_rgb(p: f64, q: f64, mut t: f64) -> f64 {
        if t < 0.0 { t += 1.0; }
        if t > 1.0 { t -= 1.0; }
        if t < 1.0 / 6.0 { p + (q - p) * 6.0 * t }
        else if t < 1.0 / 2.0 { q }
        else if t < 2.0 / 3.0 { p + (q - p) * (2.0 / 3.0 - t) * 6.0 }
        else { p }
    }
    if !color.starts_with('#') || color.len() != 7 {
        return color.to_string();
    }
    let r = u8::from_str_radix(&color[1..3], 16).unwrap_or(0) as f64 / 255.0;
    let g = u8::from_str_radix(&color[3..5], 16).unwrap_or(0) as f64 / 255.0;
    let b = u8::from_str_radix(&color[5..7], 16).unwrap_or(0) as f64 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let (h, s) = if (max - min).abs() < f64::EPSILON {
        (0.0, 0.0)
    } else {
        let d = max - min;
        let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
        let h = if (max - r).abs() < f64::EPSILON {
            (g - b) / d + if g < b { 6.0 } else { 0.0 }
        } else if (max - g).abs() < f64::EPSILON {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        (h / 6.0, s)
    };
    let l2 = (l + amount).min(1.0);
    let (r2, g2, b2) = if s.abs() < f64::EPSILON {
        (l2, l2, l2)
    } else {
        let q = if l2 < 0.5 { l2 * (1.0 + s) } else { l2 + s - l2 * s };
        let p = 2.0 * l2 - q;
        (
            hue_to_rgb(p, q, h + 1.0 / 3.0),
            hue_to_rgb(p, q, h),
            hue_to_rgb(p, q, h - 1.0 / 3.0),
        )
    };
    format!(
        "#{:02x}{:02x}{:02x}",
        (r2 * 255.0).round() as u8,
        (g2 * 255.0).round() as u8,
        (b2 * 255.0).round() as u8
    )
}

/// '#rrggbb' → 'rgba(r,g,b,a)'; anything else is returned unchanged.
#[allow(dead_code)]
fn color_rgba(color: &str, a: f64) -> String {
    if color.len() == 7 && color.starts_with('#') {
        format!(
            "rgba({},{},{},{a})",
            u8::from_str_radix(&color[1..3], 16).unwrap_or(0),
            u8::from_str_radix(&color[3..5], 16).unwrap_or(0),
            u8::from_str_radix(&color[5..7], 16).unwrap_or(0)
        )
    } else {
        color.to_string()
    }
}

/// scales.y with `stacked` and an optional min, plus themed grid/tick colors
/// for both axes.
pub fn y_scale(stacked: bool, min: Option<f64>) -> JsValue {
    fn themed_axis() -> js_sys::Object {
        let axis = js_sys::Object::new();
        let grid = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&grid, &"color".into(), &css_var("--line", "#2d3644").into());
        let _ = js_sys::Reflect::set(&axis, &"grid".into(), &grid.into());
        let border = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&border, &"color".into(), &css_var("--line", "#2d3644").into());
        let _ = js_sys::Reflect::set(&axis, &"border".into(), &border.into());
        let ticks = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&ticks, &"color".into(), &css_var("--dim", "#8b949e").into());
        let _ = js_sys::Reflect::set(&ticks, &"maxTicksLimit".into(), &10.into());
        let _ = js_sys::Reflect::set(&ticks, &"autoSkip".into(), &true.into());
        let _ = js_sys::Reflect::set(&axis, &"ticks".into(), &ticks.into());
        axis
    }
    let scales = js_sys::Object::new();
    let x = themed_axis();
    let y = themed_axis();
    let _ = js_sys::Reflect::set(&y, &"stacked".into(), &stacked.into());
    if let Some(min) = min {
        let _ = js_sys::Reflect::set(&y, &"min".into(), &min.into());
    }
    let _ = js_sys::Reflect::set(&scales, &"x".into(), &x);
    let _ = js_sys::Reflect::set(&scales, &"y".into(), &y);
    scales.into()
}
