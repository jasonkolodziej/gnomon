//! The bridge to `gnomon-web`. In the browser: an `EventSource` on
//! `/api/stream` for live state and events, and `fetch` for actions; each
//! result reaches the app through `AppHandle::update`, so the view re-runs
//! on the next frame. Natively (tests, previews) there is no server and
//! every call is a no-op that says so.

use crate::state::State;
#[cfg(target_arch = "wasm32")]
use gnomon_web_model::ActionReply;
use gnomon_web_model::{LabRequest, SubmitForm};

#[cfg(target_arch = "wasm32")]
mod web {
    use super::*;
    use gnomon_web_model::{ClusterView, EventView, LabReport};
    use rune::AppHandle;
    use std::cell::RefCell;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{EventSource, Headers, MessageEvent, RequestInit, Response};

    thread_local! {
        static HANDLE: RefCell<Option<AppHandle<State>>> = const { RefCell::new(None) };
    }

    fn update(f: impl FnOnce(&mut State) + Send + 'static) {
        HANDLE.with(|h| {
            if let Some(h) = h.borrow().as_ref() {
                h.update(move |s, _| f(s));
            }
        });
    }

    fn log(msg: &str) {
        web_sys::console::warn_1(&JsValue::from_str(msg));
    }

    /// `?demo` in the page URL: show sample data, connect to nothing.
    pub fn demo_requested() -> bool {
        web_sys::window()
            .and_then(|w| w.location().search().ok())
            .is_some_and(|s| s.contains("demo"))
    }

    pub fn connect(handle: AppHandle<State>) {
        HANDLE.with(|h| *h.borrow_mut() = Some(handle));
        let es = match EventSource::new("api/stream") {
            Ok(es) => es,
            Err(e) => {
                log(&format!("gnomon: cannot open the event stream: {e:?}"));
                return;
            }
        };
        let on_state = Closure::<dyn FnMut(MessageEvent)>::new(|m: MessageEvent| {
            let Some(data) = m.data().as_string() else {
                return;
            };
            match serde_json::from_str::<ClusterView>(&data) {
                Ok(v) => update(move |s| s.apply_view(v)),
                Err(e) => log(&format!("gnomon: bad state message: {e}")),
            }
        });
        let on_events = Closure::<dyn FnMut(MessageEvent)>::new(|m: MessageEvent| {
            let Some(data) = m.data().as_string() else {
                return;
            };
            match serde_json::from_str::<Vec<EventView>>(&data) {
                Ok(v) => update(move |s| s.push_events(v)),
                Err(e) => log(&format!("gnomon: bad events message: {e}")),
            }
        });
        let on_history = Closure::<dyn FnMut(MessageEvent)>::new(|m: MessageEvent| {
            let Some(data) = m.data().as_string() else {
                return;
            };
            match serde_json::from_str::<Vec<EventView>>(&data) {
                Ok(v) => update(move |s| s.replace_events(v)),
                Err(e) => log(&format!("gnomon: bad history message: {e}")),
            }
        });
        let on_reset = Closure::<dyn FnMut(MessageEvent)>::new(|_| update(|s| s.reset_events()));
        let on_open = Closure::<dyn FnMut()>::new(|| update(|s| s.live = true));
        // EventSource reconnects by itself; the header shows the gap.
        let on_error = Closure::<dyn FnMut()>::new(|| update(|s| s.live = false));
        let _ = es.add_event_listener_with_callback("state", on_state.as_ref().unchecked_ref());
        let _ = es.add_event_listener_with_callback("events", on_events.as_ref().unchecked_ref());
        let _ = es.add_event_listener_with_callback("history", on_history.as_ref().unchecked_ref());
        let _ = es.add_event_listener_with_callback("reset", on_reset.as_ref().unchecked_ref());
        es.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        es.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        // The page lives as long as the app: the listeners do too.
        on_state.forget();
        on_events.forget();
        on_history.forget();
        on_reset.forget();
        on_open.forget();
        on_error.forget();
        std::mem::forget(es);
    }

    async fn post(url: &str, body: String) -> Result<String, String> {
        let window = web_sys::window().ok_or("no window")?;
        let init = RequestInit::new();
        init.set_method("POST");
        let headers = Headers::new().map_err(|e| format!("{e:?}"))?;
        headers
            .set("content-type", "application/json")
            .map_err(|e| format!("{e:?}"))?;
        init.set_headers(&headers);
        init.set_body(&JsValue::from_str(&body));
        let resp = JsFuture::from(window.fetch_with_str_and_init(url, &init))
            .await
            .map_err(|e| format!("network error: {e:?}"))?;
        let resp: Response = resp.dyn_into().map_err(|_| "not a response")?;
        let text = JsFuture::from(resp.text().map_err(|e| format!("{e:?}"))?)
            .await
            .map_err(|e| format!("{e:?}"))?
            .as_string()
            .unwrap_or_default();
        if resp.ok() || resp.status() == 403 || resp.status() == 502 || resp.status() == 400 {
            Ok(text)
        } else {
            Err(format!("HTTP {}: {text}", resp.status()))
        }
    }

    fn action(url: String, body: String) {
        wasm_bindgen_futures::spawn_local(async move {
            let toast = match post(&url, body).await {
                Ok(text) => match serde_json::from_str::<ActionReply>(&text) {
                    Ok(r) => (r.message, r.ok),
                    Err(_) => (text, false),
                },
                Err(e) => (e, false),
            };
            update(move |s| s.toast = Some(toast));
        });
    }

    pub fn submit(form: SubmitForm) {
        action(
            "api/jobs".into(),
            serde_json::to_string(&form).expect("serializable"),
        );
    }

    pub fn cancel(id: u64) {
        action(format!("api/jobs/{id}/cancel"), "{}".into());
    }

    pub fn run_lab(req: LabRequest) {
        wasm_bindgen_futures::spawn_local(async move {
            let body = serde_json::to_string(&req).expect("serializable");
            let result = post("api/lab", body)
                .await
                .and_then(|t| serde_json::from_str::<LabReport>(&t).map_err(|_| t));
            update(move |s| {
                s.lab_running = false;
                match result {
                    Ok(r) => s.lab_report = Some(r),
                    Err(e) => s.toast = Some((format!("lab failed: {e}"), false)),
                }
            });
        });
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::{connect, demo_requested};

pub fn submit(s: &mut State, form: SubmitForm) {
    if s.demo {
        s.toast = Some(offline());
        return;
    }
    #[cfg(target_arch = "wasm32")]
    web::submit(form);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = form;
        s.toast = Some(offline());
    }
}

pub fn cancel(s: &mut State, id: u64) {
    if s.demo {
        s.toast = Some(offline());
        return;
    }
    #[cfg(target_arch = "wasm32")]
    web::cancel(id);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = id;
        s.toast = Some(offline());
    }
}

pub fn run_lab(s: &mut State, req: LabRequest) {
    if s.demo {
        s.lab_report = Some(crate::fixture::lab_report(&req));
        return;
    }
    s.lab_running = true;
    #[cfg(target_arch = "wasm32")]
    web::run_lab(req);
    #[cfg(not(target_arch = "wasm32"))]
    {
        s.lab_running = false;
        s.lab_report = Some(crate::fixture::lab_report(&req));
    }
}

fn offline() -> (String, bool) {
    (
        "demo data: connect gnomon-web to a controller to submit and cancel".into(),
        false,
    )
}
