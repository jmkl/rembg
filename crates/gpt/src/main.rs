use std::{io::BufRead, path::PathBuf, thread};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalPosition, LogicalSize},
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
    window::{Window, WindowId},
};
use wry::{Rect, WebView, WebViewBuilder};

const GPASTE_JS: &str = include_str!("script/gpt.js");

#[derive(Debug)]
enum UserEvent {
    Ipc(String),
    SendPrompt {
        request_id: String,
        text: String,
        image_path: Option<PathBuf>,
    },
    Abort(String),
}

struct State {
    window: Option<Window>,
    webview: Option<WebView>,
    proxy: EventLoopProxy<UserEvent>,
}

impl State {
    fn send_prompt(&self, request_id: &str, text: &str, image_path: Option<&PathBuf>) {
        let Some(wv) = &self.webview else { return };

        // Optional image -> data URL
        let image_data_url = image_path.and_then(|p| {
            let bytes = std::fs::read(p).ok()?;
            let mime = match p.extension()?.to_str()?.to_lowercase().as_str() {
                "png" => "image/png",
                "jpg" | "jpeg" => "image/jpeg",
                "webp" => "image/webp",
                "gif" => "image/gif",
                _ => return None,
            };
            Some(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
        });

        let msg = json!({
            "request_id": request_id,
            "text": text,
            "image_data_url": image_data_url,
            "image_name": "gpaste-image.png",
        });

        // serde_json output is a valid JS object literal
        let js = format!(
            "window.__gpaste ? window.__gpaste.send({msg}) : {{accepted:false,error:'bridge not loaded'}}"
        );
        let _ = wv.evaluate_script_with_callback(&js, |result| {
            // result is JSON, e.g. {"accepted":true}
            println!("send result: {result}");
        });
    }

    fn abort(&self, request_id: &str) {
        if let Some(wv) = &self.webview {
            let id = serde_json::to_string(request_id).unwrap();
            let _ = wv.evaluate_script(&format!("window.__gpaste && window.__gpaste.abort({id})"));
        }
    }

    fn handle_gpaste(&self, v: &Value) {
        let id = v["request_id"].as_str().unwrap_or("?");
        match v["event"].as_str() {
            Some("prompt_sent") => println!("[{id}] prompt sent"),
            Some("image_copied") => {
                let b64 = v["base64"].as_str().unwrap_or("");
                match STANDARD.decode(b64) {
                    Ok(png) => {
                        let path = format!("out-{id}.png");
                        std::fs::write(&path, png).unwrap();
                        println!(
                            "[{id}] image saved to {path} ({}x{})",
                            v["width"], v["height"]
                        );
                    }
                    Err(e) => eprintln!("[{id}] bad base64: {e}"),
                }
            }
            Some("bridge_error") => eprintln!("[{id}] error: {}", v["error"]),
            _ => {}
        }
    }
}

impl ApplicationHandler<UserEvent> for State {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = event_loop
            .create_window(
                Window::default_attributes()
                    .with_title("GPT")
                    .with_inner_size(LogicalSize::new(350, 600)),
            )
            .unwrap();

        let proxy = self.proxy.clone();
        let webview = WebViewBuilder::new()
            .with_devtools(true)
            .with_initialization_script(GPASTE_JS)
            .with_ipc_handler(move |req: wry::http::Request<String>| {
                // Only accept messages from the page we trust
                if !req.uri().to_string().starts_with("https://chatgpt.com") {
                    return;
                }
                let _ = proxy.send_event(UserEvent::Ipc(req.into_body()));
            })
            .with_url("https://chatgpt.com/")
            .build_as_child(&window)
            .unwrap();

        let size: LogicalSize<u32> = window.inner_size().to_logical(window.scale_factor());
        let _ = webview.set_bounds(Rect {
            position: LogicalPosition::new(0, 0).into(),
            size: LogicalSize::new(size.width, size.height).into(),
        });
        _ = webview.zoom(0.7);

        self.window = Some(window);
        self.webview = Some(webview);
    }

    fn user_event(&mut self, _el: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Ipc(body) => {
                if let Ok(v) = serde_json::from_str::<Value>(&body) {
                    if v["type"] == "gpaste" {
                        self.handle_gpaste(&v);
                    }
                }
            }
            UserEvent::SendPrompt {
                request_id,
                text,
                image_path,
            } => {
                self.send_prompt(&request_id, &text, image_path.as_ref());
            }
            UserEvent::Abort(id) => self.abort(&id),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::Resized(size) => {
                if let (Some(w), Some(wv)) = (&self.window, &self.webview) {
                    let s = size.to_logical::<u32>(w.scale_factor());
                    let _ = wv.set_bounds(Rect {
                        position: LogicalPosition::new(0, 0).into(),
                        size: LogicalSize::new(s.width, s.height).into(),
                    });
                }
            }
            WindowEvent::CloseRequested => event_loop.exit(),
            _ => {}
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let el = EventLoop::<UserEvent>::with_user_event().build()?;
    let proxy = el.create_proxy();

    // Demo trigger: type a prompt in the terminal, or "abort <id>".
    // Log in to ChatGPT in the window first.
    let p = proxy.clone();
    thread::spawn(move || {
        let mut n = 0;
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if let Some(id) = line.strip_prefix("abort ") {
                let _ = p.send_event(UserEvent::Abort(id.trim().to_string()));
                continue;
            }
            n += 1;
            let _ = p.send_event(UserEvent::SendPrompt {
                request_id: format!("req{n}"),
                text: line,
                image_path: None, // Some(PathBuf::from("input.png"))
            });
        }
    });

    let mut state = State {
        window: None,
        webview: None,
        proxy,
    };
    el.run_app(&mut state)?;
    Ok(())
}
