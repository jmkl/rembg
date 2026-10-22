#![cfg_attr(debug_assertions, allow(dead_code, unused_imports))]
use anyhow::Result;
mod handlers;
mod transport;
mod util;
use std::sync::{Arc, Mutex};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    platform::windows::WindowAttributesExtWindows,
    window::{Window, WindowId},
};
use wry::{
    Rect, WebView, WebViewBuilder,
    dpi::{LogicalPosition, LogicalSize},
};

type CmdQueue = Arc<Mutex<Vec<String>>>;

struct State {
    window: Option<Window>,
    webview: Option<WebView>,
    commands: CmdQueue,
}

impl ApplicationHandler for State {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let attributes = Window::default_attributes()
            .with_decorations(false)
            .with_undecorated_shadow(true)
            .with_inner_size(LogicalSize::new(1280, 800));
        let window = event_loop.create_window(attributes).unwrap();

        let cmds = self.commands.clone();
        let builder = WebViewBuilder::new().with_devtools(true).with_ipc_handler(
            move |req: wry::http::Request<String>| {
                cmds.lock().unwrap().push(req.into_body());
            },
        );

        // Dev mode: load Vite directly for hot reload.
        // Release: use custom protocol serving frontend/dist.
        #[cfg(debug_assertions)]
        let webview = builder
            .with_url("http://localhost:5173/")
            .build_as_child(&window)
            .unwrap();

        #[cfg(not(debug_assertions))]
        let webview = {
            let mut builder = builder;
            builder = builder.with_asynchronous_custom_protocol(
                "app".into(),
                |_id, request, responder| {
                    std::thread::spawn(move || {
                        responder.respond(transport::dispatch(&request));
                    });
                },
            );
            builder
                .with_url("app://localhost/")
                .build_as_child(&window)
                .unwrap()
        };

        // Set initial bounds.
        let size: LogicalSize<u32> = window.inner_size().to_logical(window.scale_factor());
        let _ = webview.set_bounds(Rect {
            position: LogicalPosition::new(0, 0).into(),
            size: LogicalSize::new(size.width, size.height).into(),
        });

        self.window = Some(window);
        self.webview = Some(webview);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        if let WindowEvent::Resized(size) = event
            && let (Some(window), Some(webview)) = (&self.window, &self.webview)
        {
            let size = size.to_logical::<u32>(window.scale_factor());
            let _ = webview.set_bounds(Rect {
                position: LogicalPosition::new(0, 0).into(),
                size: LogicalSize::new(size.width, size.height).into(),
            });
        }
        if let WindowEvent::CloseRequested = event {
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Drain window-control commands from the webview.
        let cmds: Vec<String> = {
            let mut q = self.commands.lock().unwrap();
            std::mem::take(&mut *q)
        };
        for cmd in cmds {
            if let Some(w) = &self.window {
                match cmd.as_str() {
                    "minimize" => w.set_minimized(true),
                    "maximize" => {
                        let is_max = w.is_maximized();
                        w.set_maximized(!is_max);
                    }
                    "close" => event_loop.exit(),
                    "drag" => {
                        let _ = w.drag_window();
                    }
                    _ => {}
                }
            }
        }
    }
}

fn main() -> Result<()> {
    handlers::files::ensure_dirs();

    // Dev-only: run a tiny HTTP server so Vite can proxy /api/* to us.
    #[cfg(debug_assertions)]
    {
        std::thread::spawn(|| {
            use std::io::Read;
            let server = match tiny_http::Server::http("127.0.0.1:8080") {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("dev http server failed: {e}");
                    return;
                }
            };
            eprintln!("dev http server on http://127.0.0.1:8080");
            for mut request in server.incoming_requests() {
                let method = request.method().as_str().to_string();
                let url = request.url().to_string();
                let mut body = Vec::new();
                let _ = request.as_reader().read_to_end(&mut body);
                let mut http_req = http::Request::builder()
                    .method(method.as_str())
                    .uri(format!("http://localhost{}", url));
                for h in request.headers().iter() {
                    http_req = http_req.header(h.field.as_str().to_string(), h.value.as_str());
                }
                let http_req = match http_req.body(body) {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                let resp = transport::dispatch(&http_req);
                let status = resp.status().as_u16();
                let mut r =
                    tiny_http::Response::from_data(resp.body().clone()).with_status_code(status);
                for (k, v) in resp.headers().iter() {
                    if let Ok(h) =
                        tiny_http::Header::from_bytes(k.as_str().as_bytes(), v.as_bytes())
                    {
                        r = r.with_header(h);
                    }
                }
                let _ = request.respond(r);
            }
        });
    }

    let event_loop = EventLoop::new()?;
    let state = State {
        window: None,
        webview: None,
        commands: Arc::new(Mutex::new(Vec::new())),
    };
    let mut state = state;
    event_loop.run_app(&mut state)?;
    Ok(())
}
