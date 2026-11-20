//! Thin wasm-bindgen shim over the vendored chart.js 4.x UMD build.

use wasm_bindgen::prelude::*;

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
        let _ = js_sys::Reflect::set(&options, &"responsive".into(), &true.into());
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
        let fill_js = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&fill_js, &"target".into(), &"origin".into());
        let bg = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&bg, &"r".into(), &color_rgba(color, 0.15).into());
        let _ = js_sys::Reflect::set(&fill_js, &"backgroundColor".into(), &bg.into());
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

/// '#rrggbb' → 'rgba(r,g,b,a)'; anything else is returned unchanged.
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

/// scales.y with `stacked` and an optional min.
pub fn y_scale(stacked: bool, min: Option<f64>) -> JsValue {
    let x = js_sys::Object::new();
    let y = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&y, &"stacked".into(), &stacked.into());
    if let Some(min) = min {
        let _ = js_sys::Reflect::set(&y, &"min".into(), &min.into());
    }
    let _ = js_sys::Reflect::set(&x, &"y".into(), &y);
    x.into()
}
